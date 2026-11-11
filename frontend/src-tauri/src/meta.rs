//! Masaüstüne özgü kalıcı bilgiler: `%LOCALAPPDATA%\kor\desktop.json`.
//! Döküm dosyaları (`sessions/*.jsonl`, CLI ile ortak) yalnızca mesajları tutar; kenar çubuğunun bölümleri,
//! sabitleme, arşiv, özel başlık, oturumun klasörü/modeli, arayüz ayarları ve kullanım kaydı burada durur.
//! Dosya atomik yazılır; bozuksa üzerine yazılmaz (kullanıcının düzenlemesi kaybolmasın).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const MAX_SECTIONS: usize = 50;
pub const MAX_SECTION_NAME: usize = 60;
pub const MAX_TITLE: usize = 200;
/// Kullanım kaydı en fazla bu kadar giriş tutar (eskiler düşer).
const MAX_USAGE: usize = 20_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Section {
    pub id: String,
    pub name: String,
    pub created_at: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct SessionMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_title: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub pinned: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub archived: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section_id: Option<String>,
    /// Sohbetin açıldığı klasör (kenar çubuğundaki proje grubu).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    /// §28.6 etiketler ve renk (`kor sessions tag|color` ile ortak, ADR 0050).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageEntry {
    /// Unix ms
    pub at: u64,
    pub model: String,
    pub prompt: u64,
    pub completion: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub model_aliases: BTreeMap<String, String>,
    pub task_models: BTreeMap<String, String>,
    pub theme: String,
    pub notifications: Value,
    pub git: Value,
    pub editor: Value,
    pub scripts: Value,
    pub profiles: Vec<Value>,
    pub logs_keep_days: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            model_aliases: BTreeMap::new(),
            task_models: BTreeMap::new(),
            theme: "system".into(),
            notifications: json!({ "desktop": true, "sound": false, "soundName": "soft" }),
            git: json!({ "branchPrefix": "kor/", "worktreeDir": "" }),
            editor: json!({ "type": "code", "customCommand": "", "sshHost": "" }),
            scripts: json!({ "setup": "", "cleanup": "", "dev": "" }),
            profiles: Vec::new(),
            logs_keep_days: 30,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Compact {
    pub max_tokens: Option<u64>,
    pub ratio: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Meta {
    pub version: u32,
    pub sections: Vec<Section>,
    pub sessions: BTreeMap<String, SessionMeta>,
    pub last_session: Option<String>,
    pub settings: Settings,
    pub output_style: String,
    pub compact: Compact,
    /// Model başına araç çağırma modu (`auto` kayıt tutulmaz).
    pub tool_modes: BTreeMap<String, String>,
    pub usage: Vec<UsageEntry>,
    /// Masaüstü sohbetlerine eklenen kullanıcı sistem istemi; boşsa yok.
    pub system_prompt: Option<String>,
    pub confidence_score: bool,
}

impl Default for Meta {
    fn default() -> Self {
        Self {
            version: 1,
            sections: Vec::new(),
            sessions: BTreeMap::new(),
            last_session: None,
            settings: Settings::default(),
            output_style: "default".into(),
            compact: Compact::default(),
            tool_modes: BTreeMap::new(),
            usage: Vec::new(),
            system_prompt: None,
            confidence_score: false,
        }
    }
}

impl Meta {
    pub fn session(&mut self, id: &str) -> &mut SessionMeta {
        self.sessions.entry(id.to_owned()).or_default()
    }

    /// Boş kalan oturum kaydı silinir; dosya büyümesin.
    pub fn tidy(&mut self, id: &str) {
        if self
            .sessions
            .get(id)
            .is_some_and(|s| *s == SessionMeta::default())
        {
            self.sessions.remove(id);
        }
    }

    pub fn record_usage(&mut self, entry: UsageEntry) {
        self.usage.push(entry);
        if self.usage.len() > MAX_USAGE {
            let drop = self.usage.len() - MAX_USAGE;
            self.usage.drain(..drop);
        }
    }
}

pub const OUTPUT_STYLES: &[&str] = &["default", "explanatory", "concise", "learning"];

/// Çıktı stilinin sistem istemine eklenen bölümü (arayüzdeki `core/output-styles.ts` ile aynı).
pub fn style_prompt(id: &str) -> Option<&'static str> {
    match id {
        "explanatory" => Some(
            "# OUTPUT STYLE: EXPLANATORY
While working, add short educational \"Neden:\" notes that explain the reasoning behind implementation choices and the codebase patterns you discover. Keep each note to 1-3 sentences, place it right after the relevant step, and never let notes delay or replace the actual work.",
        ),
        "concise" => Some(
            "# OUTPUT STYLE: CONCISE
Be as brief as possible. No preamble, no recap of the question, no closing remarks. Prefer one-line answers, bullet fragments over sentences, and code over prose. Only include what the user needs to act.",
        ),
        "learning" => Some(
            "# OUTPUT STYLE: LEARNING
Act as a patient mentor. Explain concepts step by step with small examples. For small, self-contained pieces of code (about 2-10 lines) that teach something, do not write them yourself: describe exactly what is needed and leave a clearly marked \"SENİN SIRAN:\" task for the user, then review their attempt when they reply. Still complete larger or risky parts yourself.",
        ),
        _ => None,
    }
}

pub fn path() -> Option<PathBuf> {
    kor_runtime::kor_home().map(|h| h.join("desktop.json"))
}

static LOCK: Mutex<()> = Mutex::new(());

fn load_from(p: &std::path::Path) -> Result<Meta, String> {
    match std::fs::read(p) {
        Ok(b) => serde_json::from_slice(&b)
            .map_err(|e| format!("{} bozuk, düzeltin ya da silin: {e}", p.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Meta::default()),
        Err(e) => Err(e.to_string()),
    }
}

fn save_to(p: &std::path::Path, m: &Meta) -> Result<(), String> {
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = p.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(m).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, p).map_err(|e| e.to_string())
}

/// Okur; dosya yoksa varsayılan.
pub fn read() -> Result<Meta, String> {
    let p = path().ok_or("Kullanıcı veri klasörü bulunamadı")?;
    let _g = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    load_from(&p)
}

/// Okur, değiştirir, `f` başarılıysa yazar.
pub fn update<T>(f: impl FnOnce(&mut Meta) -> Result<T, String>) -> Result<T, String> {
    let p = path().ok_or("Kullanıcı veri klasörü bulunamadı")?;
    let _g = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut m = load_from(&p)?;
    let out = f(&mut m)?;
    save_to(&p, &m)?;
    Ok(out)
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// UUID v4 biçiminde kimlik (bölümler); rastgelelik için işletim sisteminin kaynağı.
pub fn new_uuid() -> String {
    let mut b = [0u8; 16];
    getrandom_fill(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}

fn getrandom_fill(buf: &mut [u8]) {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    // RandomState işletim sisteminin rastgele kaynağından tohumlanır; bölüm kimliği için yeterli (gizli değil).
    for chunk in buf.chunks_mut(8) {
        let mut h = RandomState::new().build_hasher();
        h.write_u128(u128::from(now_ms()));
        let v = h.finish().to_le_bytes();
        chunk.copy_from_slice(&v[..chunk.len()]);
    }
}

pub fn valid_section_name(name: &str) -> Result<String, String> {
    let n = name.trim();
    if n.is_empty() {
        return Err("Bölüm adı boş olamaz".into());
    }
    if n.chars().count() > MAX_SECTION_NAME {
        // Konumsal argüman: arayüzün çeviri kalıbı `{0}` yer tutucusuyla eşleşir.
        #[allow(clippy::uninlined_format_args)]
        return Err(format!(
            "Bölüm adı en fazla {} karakter olabilir",
            MAX_SECTION_NAME
        ));
    }
    Ok(n.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_damaged_file_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("desktop.json");
        let mut m = load_from(&p).unwrap();
        m.session("a").pinned = true;
        m.sections.push(Section {
            id: new_uuid(),
            name: "İş".into(),
            created_at: 1,
        });
        save_to(&p, &m).unwrap();
        let back = load_from(&p).unwrap();
        assert!(back.sessions["a"].pinned);
        assert_eq!(back.sections[0].name, "İş");
        std::fs::write(&p, "{bozuk").unwrap();
        assert!(load_from(&p).unwrap_err().contains("bozuk"));
    }

    #[test]
    fn uuids_look_like_v4_and_differ() {
        let a = new_uuid();
        let b = new_uuid();
        assert_eq!(a.len(), 36);
        assert_eq!(&a[14..15], "4");
        assert_ne!(a, b);
    }

    #[test]
    fn tidy_drops_empty_records_and_usage_is_capped() {
        let mut m = Meta::default();
        m.session("x");
        m.tidy("x");
        assert!(m.sessions.is_empty());
        for i in 0..(MAX_USAGE + 5) {
            m.record_usage(UsageEntry {
                at: i as u64,
                model: "m".into(),
                prompt: 1,
                completion: 1,
            });
        }
        assert_eq!(m.usage.len(), MAX_USAGE);
        assert_eq!(m.usage[0].at, 5);
    }

    #[test]
    fn section_names_are_checked() {
        assert_eq!(valid_section_name("  Kişisel ").unwrap(), "Kişisel");
        assert!(valid_section_name("   ").is_err());
        assert!(valid_section_name(&"x".repeat(61)).is_err());
    }
}
