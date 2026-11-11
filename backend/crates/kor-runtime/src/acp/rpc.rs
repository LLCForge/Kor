//! JSON-RPC 2.0 over newline-delimited JSON on a byte stream, both directions at once: what ACP
//! runs on (stdio). Requests from the other side are answered by a [`Handler`] in their own
//! task; requests from this side wait for their response ([`Rpc::request`]).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncRead, AsyncWrite, AsyncWriteExt as _, BufReader};
use tokio::sync::{mpsc, oneshot};

/// Largest accepted message (a prompt with embedded files fits).
pub const MAX_LINE: usize = 64 * 1024 * 1024;

pub const PARSE_ERROR: i64 = -32_700;
pub const INVALID_REQUEST: i64 = -32_600;
pub const METHOD_NOT_FOUND: i64 = -32_601;
pub const INVALID_PARAMS: i64 = -32_602;
pub const INTERNAL_ERROR: i64 = -32_603;
pub const AUTH_REQUIRED: i64 = -32_000;
pub const RESOURCE_NOT_FOUND: i64 = -32_002;

#[derive(Debug, Clone, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    #[must_use]
    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }

    fn to_json(&self) -> Value {
        let mut e = json!({ "code": self.code, "message": self.message });
        if let Some(d) = &self.data {
            e["data"] = d.clone();
        }
        e
    }
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}

type Pending = Mutex<HashMap<u64, oneshot::Sender<Result<Value, RpcError>>>>;

/// One end of a connection.
#[derive(Debug)]
pub struct Rpc {
    out: mpsc::UnboundedSender<Value>,
    pending: Pending,
    next: AtomicU64,
}

impl Rpc {
    /// Call `method` on the other side and wait for its answer.
    ///
    /// # Errors
    /// The other side's error, or `INTERNAL_ERROR` when the connection closed.
    pub async fn request(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, tx);
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        rx.await
            .unwrap_or_else(|_| Err(RpcError::new(INTERNAL_ERROR, "connection closed")))
    }

    #[allow(clippy::needless_pass_by_value)] // callers build the params inline
    pub fn notify(&self, method: &str, params: Value) {
        self.send(json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    fn send(&self, v: Value) {
        let _ = self.out.send(v);
    }

    fn answer(&self, id: &Value, r: Result<Value, RpcError>) {
        self.send(match r {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(e) => json!({ "jsonrpc": "2.0", "id": id, "error": e.to_json() }),
        });
    }

    fn resolve(&self, v: &Value) {
        let Some(id) = v["id"].as_u64() else { return };
        let tx = self
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id);
        if let Some(tx) = tx {
            let r = match v.get("error") {
                Some(e) if !e.is_null() => Err(RpcError {
                    code: e["code"].as_i64().unwrap_or(INTERNAL_ERROR),
                    message: e["message"].as_str().unwrap_or("error").to_owned(),
                    data: e.get("data").cloned(),
                }),
                _ => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
            };
            let _ = tx.send(r);
        }
    }
}

/// Answers the other side's requests and notifications.
#[async_trait]
pub trait Handler: Send + Sync {
    async fn request(&self, rpc: Arc<Rpc>, method: &str, params: Value) -> Result<Value, RpcError>;
    async fn notification(&self, rpc: Arc<Rpc>, method: &str, params: Value);
}

/// A connection over `reader`/`writer`. Returns the [`Rpc`] for calls from this side and a
/// future that runs until the other side closes its end.
pub fn connect<R, W>(
    reader: R,
    writer: W,
    handler: Arc<dyn Handler>,
) -> (Arc<Rpc>, impl std::future::Future<Output = ()> + Send)
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (out, mut out_rx) = mpsc::unbounded_channel::<Value>();
    let rpc = Arc::new(Rpc {
        out,
        pending: Mutex::new(HashMap::new()),
        next: AtomicU64::new(1),
    });
    let r = rpc.clone();
    let run = async move {
        let mut writer = writer;
        let write = tokio::spawn(async move {
            while let Some(v) = out_rx.recv().await {
                let mut line = v.to_string();
                line.push('\n');
                if writer.write_all(line.as_bytes()).await.is_err() || writer.flush().await.is_err() {
                    break;
                }
            }
        });
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if line.trim().is_empty() {
                continue;
            }
            if line.len() > MAX_LINE {
                r.answer(&Value::Null, Err(RpcError::new(INVALID_REQUEST, "message too large")));
                continue;
            }
            let v: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(e) => {
                    r.answer(&Value::Null, Err(RpcError::new(PARSE_ERROR, e.to_string())));
                    continue;
                }
            };
            let method = v["method"].as_str().map(str::to_owned);
            match (method, v.get("id").cloned()) {
                (Some(m), Some(id)) if !id.is_null() => {
                    let (h, r2) = (handler.clone(), r.clone());
                    let params = v.get("params").cloned().unwrap_or(Value::Null);
                    tokio::spawn(async move {
                        let res = h.request(r2.clone(), &m, params).await;
                        r2.answer(&id, res);
                    });
                }
                (Some(m), _) => {
                    let params = v.get("params").cloned().unwrap_or(Value::Null);
                    handler.notification(r.clone(), &m, params).await;
                }
                (None, Some(_)) => r.resolve(&v),
                (None, None) => {
                    r.answer(&Value::Null, Err(RpcError::new(INVALID_REQUEST, "not a JSON-RPC message")));
                }
            }
        }
        // Fail whatever still waits for an answer.
        r.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        drop(r);
        // The writer ends once every `Rpc` handle is gone (answers still in flight get out).
        drop(write);
    };
    (rpc, run)
}
