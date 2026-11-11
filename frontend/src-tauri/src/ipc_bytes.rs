//! `kor_bytes`: büyük yükler JSON yerine ham bayt (§32.2 "IPC yalın", ADR 0062). Dosya içeriği ve
//! belge/görsel önizlemesi `tauri::ipc::Response` ile ikili gider; arayüz `ArrayBuffer` alır
//! (kor-bridge.ts `BINARY`). base64 + JSON'un 4/3 büyümesi ve iki kez ayrıştırma yoktur.
//! Okuma `spawn_blocking` içinde: tokio işçisi diskte beklemez.

use serde_json::Value;
use tauri::State;
use tauri::ipc::Response;

use crate::state::AppState;

#[tauri::command]
pub async fn kor_bytes(
    method: String,
    args: Vec<Value>,
    state: State<'_, AppState>,
) -> Result<Response, String> {
    let rel = args
        .first()
        .and_then(Value::as_str)
        .ok_or_else(|| format!("`{method}`: 1. bağımsız değişken metin olmalı"))?
        .to_owned();
    let root = crate::api::root(&state)?;
    let read: fn(&std::path::Path, &str) -> Result<Vec<u8>, String> = match method.as_str() {
        "readFile" => crate::store::read_file_bytes,
        "g21ReadDoc" => crate::api_files::read_doc,
        _ => return Err(format!("`{method}`: ikili yanıtı olan bir yöntem değil")),
    };
    let bytes = tauri::async_runtime::spawn_blocking(move || read(&root, &rel))
        .await
        .map_err(|e| e.to_string())??;
    Ok(Response::new(bytes))
}

#[cfg(test)]
mod tests {

    #[test]
    fn binary_methods_read_the_same_bytes_as_their_json_twins() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "merhaba\u{0}ş").unwrap();
        let raw = crate::store::read_file_bytes(dir.path(), "a.txt").unwrap();
        assert_eq!(raw, "merhaba\u{0}ş".as_bytes());
        assert_eq!(
            crate::store::read_file(dir.path(), "a.txt").unwrap(),
            String::from_utf8_lossy(&raw)
        );
        assert!(crate::store::read_file_bytes(dir.path(), "../a.txt").is_err());
    }
}
