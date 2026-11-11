//! `kor-sandbox:` şeması: HTML/SVG önizlemelerinin yüklendiği alıcı belge (bkz. `SandboxedPreview.tsx`).
//!
//! iframe `sandbox="allow-scripts"` ile (allow-same-origin YOK) yüklenir; belge kendi CSP başlığını taşır:
//! ağ tamamen kapalı, satır içi stil açık. `static` modda yalnızca alıcı betik çalışır (her yanıtta yeni
//! nonce); `scripts` modda kullanıcı izin verdiği için satır içi betikler de çalışır, dış betik yine yok.
//! İçerik arayüzden `postMessage` ile gelir; alıcı yalnızca ilk mesajı ve yalnızca üst pencereden kabul eder.

use tauri::http::{Request, Response, StatusCode};

const RECEIVER: &str = r"(function () {
  var SRC = 'g-rich-preview', HOST = 'g-rich-host', done = false;
  function report() {
    parent.postMessage({ source: SRC, height: document.documentElement.scrollHeight }, '*');
  }
  addEventListener('message', function (e) {
    if (done || e.source !== parent) return;
    var d = e.data;
    if (!d || d.source !== HOST || typeof d.html !== 'string') return;
    done = true;
    if (typeof d.bg === 'string') document.body.style.background = d.bg;
    if (typeof d.fg === 'string') document.body.style.color = d.fg;
    var doc = new DOMParser().parseFromString(d.html, 'text/html');
    doc.querySelectorAll('meta[http-equiv], base').forEach(function (n) { n.remove(); });
    var nodes = Array.prototype.slice.call(doc.head.childNodes).concat(Array.prototype.slice.call(doc.body.childNodes));
    nodes.forEach(function (n) { document.body.appendChild(document.adoptNode(n)); });
    if (SCRIPTS) {
      document.body.querySelectorAll('script').forEach(function (old) {
        var s = document.createElement('script');
        for (var i = 0; i < old.attributes.length; i++) s.setAttribute(old.attributes[i].name, old.attributes[i].value);
        s.text = old.text;
        old.replaceWith(s);
      });
    }
    report();
    if (window.ResizeObserver) new ResizeObserver(report).observe(document.body);
  });
  // Bağlantılar çerçeveyi başka adrese götürmesin.
  document.addEventListener('click', function (e) {
    var a = e.target && e.target.closest ? e.target.closest('a[href], area[href]') : null;
    if (a) e.preventDefault();
  }, true);
  parent.postMessage({ source: SRC, ready: true }, '*');
})();";

fn nonce() -> String {
    use std::hash::{BuildHasher, Hasher};
    // Her RandomState işletim sisteminden tohumlanır; iki karışım 128 bitlik tek kullanımlık değer verir.
    let mut out = String::new();
    for i in 0..2u8 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u8(i);
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

fn csp(scripts: bool, nonce: &str) -> String {
    let script = if scripts {
        "'unsafe-inline'".to_owned()
    } else {
        format!("'nonce-{nonce}'")
    };
    format!(
        "default-src 'none'; script-src {script}; style-src 'unsafe-inline'; img-src data: blob:; \
         font-src data:; media-src data: blob:; connect-src 'none'; frame-src 'none'; \
         worker-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'"
    )
}

/// Arayüzün `sandboxUrl()` adresleri: `/preview/static`, `/preview/scripts`.
pub fn respond(req: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    // Windows: http://kor-sandbox.localhost/preview/static; diğerleri: kor-sandbox://preview/static
    // (orada "preview" ana bilgisayar adıdır, yol yalnızca /static).
    let path = req.uri().path();
    let scripts = match path.strip_prefix("/preview").unwrap_or(path) {
        "/static" => false,
        "/scripts" => true,
        _ => {
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Vec::new())
                .unwrap_or_default();
        }
    };
    let n = nonce();
    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><style>html,body{{margin:0;padding:8px;\
         font-family:system-ui,sans-serif;overflow-wrap:anywhere}}</style></head><body>\
         <script nonce=\"{n}\">var SCRIPTS = {scripts};\n{RECEIVER}</script></body></html>"
    );
    Response::builder()
        .header("Content-Type", "text/html; charset=utf-8")
        .header("Content-Security-Policy", csp(scripts, &n))
        .header("X-Content-Type-Options", "nosniff")
        .header("Referrer-Policy", "no-referrer")
        .header("Cache-Control", "no-store")
        .body(html.into_bytes())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get(path: &str) -> Response<Vec<u8>> {
        respond(
            &Request::builder()
                .uri(format!("kor-sandbox://preview{path}"))
                .body(Vec::new())
                .unwrap(),
        )
    }

    #[test]
    fn static_mode_allows_only_the_nonced_receiver() {
        let r = get("/static");
        let csp = r.headers()["Content-Security-Policy"]
            .to_str()
            .unwrap()
            .to_owned();
        assert!(csp.contains("script-src 'nonce-") && !csp.contains("unsafe-inline'; style"));
        assert!(csp.contains("connect-src 'none'"));
        let body = String::from_utf8(r.into_body()).unwrap();
        let n = csp
            .split("'nonce-")
            .nth(1)
            .unwrap()
            .split('\'')
            .next()
            .unwrap();
        assert_eq!(n.len(), 32);
        assert!(body.contains(&format!("nonce=\"{n}\"")) && body.contains("var SCRIPTS = false"));
        assert_ne!(nonce(), nonce());
    }

    #[test]
    fn scripts_mode_and_unknown_paths() {
        let r = get("/scripts");
        assert!(
            r.headers()["Content-Security-Policy"]
                .to_str()
                .unwrap()
                .contains("script-src 'unsafe-inline'")
        );
        assert_eq!(get("/other").status(), StatusCode::NOT_FOUND);
        let win = Request::builder()
            .uri("http://kor-sandbox.localhost/preview/static")
            .body(Vec::new())
            .unwrap();
        assert_eq!(respond(&win).status(), StatusCode::OK);
    }
}
