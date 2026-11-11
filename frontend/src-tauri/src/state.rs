//! Pencere ömrü boyunca tutulan durum: açık çalışma klasörü, son kullanılanlar, arayüzde seçilen model/mod
//! ve sohbetler.

use std::path::PathBuf;
use std::sync::Mutex;

use kor_core::PermissionMode;

use crate::chat::Chats;

const MAX_RECENT: usize = 10;

#[derive(Default)]
pub struct AppState {
    inner: Mutex<Inner>,
    pub chats: Chats,
}

#[derive(Default)]
struct Inner {
    root: Option<PathBuf>,
    // shortcut: son klasörler yalnızca bellekte; uygulama kapanınca sıfırlanır. kor-config'e yazılınca kalıcı olur.
    recent: Vec<String>,
    /// Arayüzde seçilen `sağlayıcı/model`; yoksa yapılandırmadaki `agent.model`.
    model: Option<String>,
    /// Arayüzde seçilen izin modu; yoksa yapılandırma ve uyumluluk ayarları.
    mode: Option<PermissionMode>,
}

impl AppState {
    pub fn root(&self) -> Option<PathBuf> {
        self.lock().root.clone()
    }

    /// Ajanın çalışacağı kök: açık klasör, yoksa `Belgeler/Kor` (CLI'daki gibi bir proje kökü olmadan araçlar
    /// kullanıcının tüm ev dizinine yayılmasın diye ayrı, boş bir klasör).
    pub fn workspace(&self) -> Result<PathBuf, String> {
        if let Some(r) = self.root() {
            return Ok(r);
        }
        let dir = dirs::document_dir()
            .or_else(dirs::home_dir)
            .ok_or("Varsayılan çalışma klasörü bulunamadı")?
            .join("Kor");
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("Çalışma klasörü oluşturulamadı: {e}"))?;
        dir.canonicalize().map_err(|e| e.to_string())
    }

    pub fn recent(&self) -> Vec<String> {
        self.lock().recent.clone()
    }

    /// Klasörü açar ve son kullanılanların başına alır.
    pub fn open(&self, dir: PathBuf) {
        let mut g = self.lock();
        let s = dir.to_string_lossy().into_owned();
        g.recent.retain(|r| r != &s);
        g.recent.insert(0, s);
        g.recent.truncate(MAX_RECENT);
        g.root = Some(dir);
    }

    pub fn close(&self) {
        self.lock().root = None;
    }

    pub fn model(&self) -> Option<String> {
        self.lock().model.clone()
    }

    pub fn set_model(&self, model: String) {
        self.lock().model = Some(model);
    }

    pub fn mode(&self) -> Option<PermissionMode> {
        self.lock().mode
    }

    pub fn set_mode(&self, mode: PermissionMode) {
        self.lock().mode = Some(mode);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        // Zehirlenmiş kilit yalnızca bir panikten sonra olur; durum basit alanlardır, okumaya devam etmek güvenli.
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}
