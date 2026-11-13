//! §12.1 WASM plugins: a directory with `plugin.toml` and a WebAssembly module that wasmtime runs
//! with no WASI (no files, network or clock of its own), a fuel budget and a memory cap. What a
//! plugin may do beyond computing is a capability the user grants at install time:
//!
//! - `fs.read`: read a file of the open project (`kor.read_file`),
//! - `tool:<Name>`: call that Kor tool (`kor.call_tool`; the host decides which tools it offers),
//! - `event.emit`: send an event line to Kor (`kor.emit`).
//!
//! The module must be signed: `<module>.minisig` verifies against the manifest's `public_key`
//! (minisign, as Kor's own updates). The key is pinned at the first install; a later install with
//! another key is refused until the plugin is uninstalled. Grants live in `<root>/grants.json`.
//!
//! Guest ABI (core module): exports `memory`, `kor_alloc(len) -> ptr` (where Kor writes host
//! data), and `kor_run(ptr, len) -> packed` and/or `kor_skill() -> packed`, where `packed` is
//! `(ptr << 32) | len`. Imports from module `kor`: `read_file(ptr, len) -> packed`,
//! `call_tool(name_ptr, name_len, input_ptr, input_len) -> packed`, `emit(ptr, len)`. A host call
//! returns [`DENIED`] when the capability is not granted and [`FAILED`] when it fails.
//!
//! A Claude Code skill folder (`SKILL.md` and its files) installs unchanged: [`pack_skill`] wraps
//! `SKILL.md` in a module exporting `kor_skill`, and installing any plugin with that export writes
//! the text it returns to `skills/<name>/SKILL.md`, where the agent's skill loader finds it.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use wasmtime::{
    Caller, Config, Engine, Extern, Instance, Linker, Module, Store, StoreLimits,
    StoreLimitsBuilder,
};

use crate::plugins::{copy_dir, safe};

pub const MANIFEST: &str = "plugin.toml";
pub const FS_READ: &str = "fs.read";
pub const EVENT_EMIT: &str = "event.emit";
pub const TOOL_PREFIX: &str = "tool:";
/// Host call result: capability not granted.
pub const DENIED: i64 = -1;
/// Host call result: granted, but the call failed (missing file, tool error, bad guest memory).
pub const FAILED: i64 = -2;

const DEFAULT_FUEL: u64 = 1_000_000_000;
const MAX_FUEL: u64 = 20_000_000_000;
const DEFAULT_MEMORY_MB: u64 = 64;
const MAX_MEMORY_MB: u64 = 512;
/// Largest input, output, file, tool result or event passed across the boundary.
const MAX_IO: usize = 8 << 20;
const MAX_SKILL: usize = 1 << 20;

/// `plugin.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    /// Module file next to the manifest: binary `.wasm` or text `.wat`.
    #[serde(default = "default_module")]
    pub module: String,
    /// minisign public key (base64) that signs the module.
    #[serde(default)]
    pub public_key: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub limits: Limits,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    pub fuel: Option<u64>,
    pub memory_mb: Option<u64>,
}

fn default_module() -> String {
    "plugin.wasm".into()
}

/// Whether `cap` is a capability Kor knows.
pub fn known_capability(cap: &str) -> bool {
    cap == FS_READ
        || cap == EVENT_EMIT
        || cap.strip_prefix(TOOL_PREFIX).is_some_and(|t| {
            !t.is_empty()
                && t.len() <= 64
                && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
}

/// A plugin name is its folder under the plugin root: not hidden, not a folder Kor uses itself.
fn plugin_name_ok(name: &str) -> bool {
    safe(name)
        && !name.starts_with('.')
        && !name.contains(':')
        && !matches!(name, "cache" | "marketplaces")
}

/// Read and check `<dir>/plugin.toml`.
///
/// # Errors
/// If the file is missing or invalid, the name or module path is unsafe, or a capability is unknown.
pub fn read_manifest(dir: &Path) -> Result<Manifest, String> {
    let text = fs::read_to_string(dir.join(MANIFEST)).map_err(|e| format!("{MANIFEST}: {e}"))?;
    let m: Manifest = toml::from_str(&text).map_err(|e| format!("{MANIFEST}: {e}"))?;
    if !plugin_name_ok(&m.name) {
        return Err(format!("{MANIFEST}: invalid name `{}`", m.name));
    }
    if !safe(&m.module) || m.module.starts_with('.') {
        return Err(format!("{MANIFEST}: module must be a file name next to it"));
    }
    if let Some(bad) = m.capabilities.iter().find(|c| !known_capability(c)) {
        return Err(format!("{MANIFEST}: unknown capability `{bad}`"));
    }
    Ok(m)
}

/// Check `<module>.minisig` against the manifest's public key.
///
/// # Errors
/// If the key or signature is missing or unreadable, or the signature does not verify.
pub fn verify_signature(dir: &Path, m: &Manifest) -> Result<(), String> {
    let key = m
        .public_key
        .as_deref()
        .filter(|k| !k.trim().is_empty())
        .ok_or("the plugin is not signed: plugin.toml has no public_key")?;
    let data = fs::read(dir.join(&m.module)).map_err(|e| format!("{}: {e}", m.module))?;
    let sig_file = format!("{}.minisig", m.module);
    let sig = fs::read_to_string(dir.join(&sig_file)).map_err(|e| format!("{sig_file}: {e}"))?;
    let pk = minisign_verify::PublicKey::from_base64(key.trim())
        .map_err(|e| format!("public_key: {e}"))?;
    let sig = minisign_verify::Signature::decode(&sig).map_err(|e| format!("{sig_file}: {e}"))?;
    pk.verify(&data, &sig, false)
        .map_err(|e| format!("signature does not verify: {e}"))
}

/// What the user allowed one installed plugin.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    /// Key pinned at install; `None` for a skill Kor packed itself from local files.
    pub public_key: Option<String>,
    pub capabilities: Vec<String>,
}

fn grants_path(root: &Path) -> PathBuf {
    root.join("grants.json")
}

/// Every stored grant, by plugin id.
pub fn grants(root: &Path) -> BTreeMap<String, Grant> {
    fs::read_to_string(grants_path(root))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_grants(root: &Path, all: &BTreeMap<String, Grant>) -> Result<(), String> {
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(all).map_err(|e| e.to_string())?;
    fs::write(grants_path(root), text).map_err(|e| e.to_string())
}

/// Take back one capability (or all of them with `None`). Returns what stays granted.
///
/// # Errors
/// If the plugin has no grant, does not hold that capability, or the file cannot be written.
pub fn revoke(root: &Path, id: &str, cap: Option<&str>) -> Result<Vec<String>, String> {
    let mut all = grants(root);
    let g = all
        .get_mut(id)
        .ok_or_else(|| format!("no grants stored for `{id}`"))?;
    match cap {
        Some(c) => {
            let before = g.capabilities.len();
            g.capabilities.retain(|x| x != c);
            if g.capabilities.len() == before {
                return Err(format!("`{id}` does not hold `{c}`"));
            }
        }
        None => g.capabilities.clear(),
    }
    let left = g.capabilities.clone();
    save_grants(root, &all)?;
    Ok(left)
}

/// Forget a plugin's grant (uninstall).
///
/// # Errors
/// If the grants file cannot be written.
pub fn forget(root: &Path, id: &str) -> Result<(), String> {
    let mut all = grants(root);
    if all.remove(id).is_some() {
        save_grants(root, &all)?;
    }
    Ok(())
}

/// Install a local directory into `<root>/<name>`: a signed WASM plugin (`plugin.toml`), or a
/// Claude Code skill folder (`SKILL.md`), which is packed into a WASM plugin first. `approve`
/// sees the manifest (capabilities, key) and decides. Returns the plugin id.
///
/// # Errors
/// If the directory is neither, the signature or key check fails, the module does not load, or
/// the user declines.
pub fn install(
    root: &Path,
    src: &Path,
    approve: &mut dyn FnMut(&Manifest) -> bool,
) -> Result<String, String> {
    if src.join(MANIFEST).is_file() {
        let m = read_manifest(src)?;
        check(root, &m.name, src, &m, true, approve)?;
        let dest = root.join(&m.name);
        replace_dir(src, &dest)?;
        commit(root, &m.name, &dest, &m)?;
        return Ok(m.name);
    }
    if !src.join("SKILL.md").is_file() {
        return Err(format!(
            "{} has neither {MANIFEST} nor SKILL.md",
            src.display()
        ));
    }
    let name = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !plugin_name_ok(&name) {
        return Err(format!("invalid skill folder name `{name}`"));
    }
    // Staged under a dot name, which `plugins::installed` skips, until it is approved.
    let staging = root.join(format!(".staging-{name}"));
    let _ = fs::remove_dir_all(&staging);
    let staged = pack_skill(src, &staging).and_then(|m| {
        // Packed here from the user's own files: Kor wrote the module, so there is no signer.
        check(root, &name, &staging, &m, false, approve)?;
        Ok(m)
    });
    let m = match staged {
        Ok(m) => m,
        Err(e) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(e);
        }
    };
    let dest = root.join(&name);
    if dest.exists() {
        fs::remove_dir_all(&dest).map_err(|e| e.to_string())?;
    }
    fs::rename(&staging, &dest).map_err(|e| e.to_string())?;
    commit(root, &name, &dest, &m)?;
    Ok(name)
}

/// After a marketplace install copied `dir`: when it is a WASM plugin, verify, ask and record the
/// grant. `Ok(false)` = not a WASM plugin (nothing to do).
///
/// # Errors
/// As [`install`]; the caller removes the copy.
pub fn admit(
    root: &Path,
    id: &str,
    dir: &Path,
    approve: &mut dyn FnMut(&Manifest) -> bool,
) -> Result<bool, String> {
    if !dir.join(MANIFEST).is_file() {
        return Ok(false);
    }
    let m = read_manifest(dir)?;
    check(root, id, dir, &m, true, approve)?;
    commit(root, id, dir, &m)?;
    Ok(true)
}

fn replace_dir(src: &Path, dest: &Path) -> Result<(), String> {
    // Re-installing the installed copy itself: deleting `dest` would delete the source.
    if fs::canonicalize(src).ok() == fs::canonicalize(dest).ok() && dest.exists() {
        return Ok(());
    }
    if dest.exists() {
        fs::remove_dir_all(dest).map_err(|e| e.to_string())?;
    }
    copy_dir(src, dest)
}

fn check(
    root: &Path,
    id: &str,
    dir: &Path,
    m: &Manifest,
    signed: bool,
    approve: &mut dyn FnMut(&Manifest) -> bool,
) -> Result<(), String> {
    if signed {
        verify_signature(dir, m)?;
    }
    if let Some(old) = grants(root).get(id)
        && old.public_key.is_some()
        && old.public_key != m.public_key
    {
        return Err(format!(
            "`{id}` is signed with a different key than the installed one; uninstall it first"
        ));
    }
    load_module(&engine()?, dir, m)?;
    if approve(m) {
        Ok(())
    } else {
        Err("installation declined".into())
    }
}

fn commit(root: &Path, id: &str, dir: &Path, m: &Manifest) -> Result<(), String> {
    let mut all = grants(root);
    all.insert(
        id.to_owned(),
        Grant {
            public_key: m.public_key.clone(),
            capabilities: m.capabilities.clone(),
        },
    );
    save_grants(root, &all)?;
    if let Some(text) = skill_text(dir, m)? {
        let skill = dir.join("skills").join(&m.name);
        fs::create_dir_all(&skill).map_err(|e| e.to_string())?;
        fs::write(skill.join("SKILL.md"), text).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Wrap a Claude Code skill folder into a WASM plugin in `out`: `SKILL.md` becomes the data of a
/// module exporting `kor_skill`; the folder's other files go to `skills/<name>/` as they are.
///
/// # Errors
/// If `SKILL.md` is missing, too large, or a file cannot be written.
pub fn pack_skill(skill_dir: &Path, out: &Path) -> Result<Manifest, String> {
    let name = skill_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !plugin_name_ok(&name) {
        return Err(format!("invalid skill folder name `{name}`"));
    }
    let text = fs::read(skill_dir.join("SKILL.md")).map_err(|e| format!("SKILL.md: {e}"))?;
    if text.len() > MAX_SKILL {
        return Err("SKILL.md is larger than 1 MiB".into());
    }
    let files = out.join("skills").join(&name);
    copy_dir(skill_dir, &files)?;
    fs::remove_file(files.join("SKILL.md")).map_err(|e| e.to_string())?;
    let wasm = wat::parse_str(skill_wat(&text)).map_err(|e| e.to_string())?;
    fs::write(out.join("plugin.wasm"), wasm).map_err(|e| e.to_string())?;
    let m = Manifest {
        name,
        version: "0.0.0".into(),
        description: "Claude Code skill packed by Kor".into(),
        module: default_module(),
        public_key: None,
        capabilities: Vec::new(),
        limits: Limits::default(),
    };
    let toml = toml::to_string(&m).map_err(|e| e.to_string())?;
    fs::write(out.join(MANIFEST), toml).map_err(|e| e.to_string())?;
    Ok(m)
}

fn skill_wat(text: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut data = String::with_capacity(text.len() * 3);
    for b in text {
        let _ = write!(data, "\\{b:02x}");
    }
    let pages = text.len().div_ceil(65_536).max(1);
    format!(
        "(module (memory (export \"memory\") {pages}) (data (i32.const 0) \"{data}\") \
         (func (export \"kor_skill\") (result i64) (i64.const {})))",
        text.len()
    )
}

/// The skill text a plugin exports through `kor_skill`, run with no capabilities.
///
/// # Errors
/// If the module fails to load or traps.
pub fn skill_text(dir: &Path, m: &Manifest) -> Result<Option<String>, String> {
    let engine = engine()?;
    let module = load_module(&engine, dir, m)?;
    if !module.exports().any(|e| e.name() == "kor_skill") {
        return Ok(None);
    }
    let (mut store, inst) = instantiate(&engine, &module, m, &[], Host::denied())?;
    let f = inst
        .get_typed_func::<(), i64>(&mut store, "kor_skill")
        .map_err(|e| format!("kor_skill: {e}"))?;
    let packed = f.call(&mut store, ()).map_err(|e| trap(&store, &e))?;
    let bytes = read_packed(&mut store, &inst, packed)?;
    if bytes.len() > MAX_SKILL {
        return Err("skill text is larger than 1 MiB".into());
    }
    Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
}

/// Host side of `kor.read_file`: project-relative path to bytes.
pub type ReadFile = Box<dyn FnMut(&str) -> Result<Vec<u8>, String>>;
/// Host side of `kor.call_tool`: tool name and JSON input to the tool's output.
pub type CallTool = Box<dyn FnMut(&str, &str) -> Result<String, String>>;

/// What a running plugin may reach. Each call is made only when its capability is granted.
pub struct Host {
    pub read_file: ReadFile,
    pub call_tool: CallTool,
    pub emit: Box<dyn FnMut(&str)>,
}

impl std::fmt::Debug for Host {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Host")
    }
}

impl Host {
    /// A host that offers nothing.
    pub fn denied() -> Self {
        Self {
            read_file: Box::new(|_| Err("no file access".into())),
            call_tool: Box::new(|_, _| Err("no tools".into())),
            emit: Box::new(|_| {}),
        }
    }
}

struct State {
    host: Host,
    caps: Vec<String>,
    limits: StoreLimits,
}

/// Run the plugin's `kor_run` with `input`. Only capabilities both requested in the manifest and
/// still granted reach the host.
///
/// # Errors
/// If the module does not load, has no `kor_run`, traps, or runs out of fuel or memory.
pub fn run(
    dir: &Path,
    m: &Manifest,
    granted: &[String],
    host: Host,
    input: &[u8],
) -> Result<Vec<u8>, String> {
    if input.len() > MAX_IO {
        return Err("input is too large".into());
    }
    let engine = engine()?;
    let module = load_module(&engine, dir, m)?;
    let caps: Vec<String> = m
        .capabilities
        .iter()
        .filter(|c| granted.contains(c))
        .cloned()
        .collect();
    let (mut store, inst) = instantiate(&engine, &module, m, &caps, host)?;
    let f = inst
        .get_typed_func::<(i32, i32), i64>(&mut store, "kor_run")
        .map_err(|e| format!("kor_run: {e}"))?;
    let (ptr, len) = if input.is_empty() {
        (0, 0)
    } else {
        let packed = put(&mut store, &inst, input)?;
        unpack(packed).ok_or("kor_alloc returned an invalid pointer")?
    };
    let packed = f
        .call(&mut store, (to_i32(ptr)?, to_i32(len)?))
        .map_err(|e| trap(&store, &e))?;
    read_packed(&mut store, &inst, packed)
}

fn engine() -> Result<Engine, String> {
    let mut cfg = Config::new();
    cfg.consume_fuel(true);
    Engine::new(&cfg).map_err(|e| e.to_string())
}

fn load_module(engine: &Engine, dir: &Path, m: &Manifest) -> Result<Module, String> {
    let bytes = fs::read(dir.join(&m.module)).map_err(|e| format!("{}: {e}", m.module))?;
    let module = Module::new(engine, bytes).map_err(|e| format!("{}: {e}", m.module))?;
    // No WASI or other host modules: only Kor's three calls exist.
    if let Some(i) = module
        .imports()
        .find(|i| i.module() != "kor" || !matches!(i.name(), "read_file" | "call_tool" | "emit"))
    {
        return Err(format!(
            "{}: imports `{}.{}`, which Kor does not provide",
            m.module,
            i.module(),
            i.name()
        ));
    }
    Ok(module)
}

fn instantiate(
    engine: &Engine,
    module: &Module,
    m: &Manifest,
    caps: &[String],
    host: Host,
) -> Result<(Store<State>, Instance), String> {
    let mem_mb = m
        .limits
        .memory_mb
        .unwrap_or(DEFAULT_MEMORY_MB)
        .min(MAX_MEMORY_MB);
    let limits = StoreLimitsBuilder::new()
        .memory_size(usize::try_from(mem_mb << 20).unwrap_or(usize::MAX))
        .instances(1)
        .memories(1)
        .tables(4)
        .build();
    let mut store = Store::new(
        engine,
        State {
            host,
            caps: caps.to_vec(),
            limits,
        },
    );
    store.limiter(|s| &mut s.limits);
    store
        .set_fuel(m.limits.fuel.unwrap_or(DEFAULT_FUEL).min(MAX_FUEL))
        .map_err(|e| e.to_string())?;
    let mut linker: Linker<State> = Linker::new(engine);
    linker
        .func_wrap(
            "kor",
            "read_file",
            |mut c: Caller<'_, State>, p: i32, l: i32| -> i64 {
                if !c.data().caps.iter().any(|x| x == FS_READ) {
                    return DENIED;
                }
                let Some(path) = guest_str(&mut c, p, l) else {
                    return FAILED;
                };
                match (c.data_mut().host.read_file)(&path) {
                    Ok(bytes) if bytes.len() <= MAX_IO => give(&mut c, &bytes),
                    _ => FAILED,
                }
            },
        )
        .map_err(|e| e.to_string())?;
    linker
        .func_wrap(
            "kor",
            "call_tool",
            |mut c: Caller<'_, State>, np: i32, nl: i32, ip: i32, il: i32| -> i64 {
                let Some(name) = guest_str(&mut c, np, nl) else {
                    return FAILED;
                };
                let cap = format!("{TOOL_PREFIX}{name}");
                if !c.data().caps.contains(&cap) {
                    return DENIED;
                }
                let Some(input) = guest_str(&mut c, ip, il) else {
                    return FAILED;
                };
                match (c.data_mut().host.call_tool)(&name, &input) {
                    Ok(out) if out.len() <= MAX_IO => give(&mut c, out.as_bytes()),
                    _ => FAILED,
                }
            },
        )
        .map_err(|e| e.to_string())?;
    linker
        .func_wrap(
            "kor",
            "emit",
            |mut c: Caller<'_, State>, p: i32, l: i32| -> i64 {
                if !c.data().caps.iter().any(|x| x == EVENT_EMIT) {
                    return DENIED;
                }
                let Some(ev) = guest_str(&mut c, p, l) else {
                    return FAILED;
                };
                (c.data_mut().host.emit)(&ev);
                0
            },
        )
        .map_err(|e| e.to_string())?;
    let inst = linker
        .instantiate(&mut store, module)
        .map_err(|e| format!("{}: {e}", m.module))?;
    Ok((store, inst))
}

fn trap(store: &Store<State>, e: &wasmtime::Error) -> String {
    if store.get_fuel().is_ok_and(|f| f == 0) {
        "the plugin ran out of fuel (CPU budget)".into()
    } else {
        format!("the plugin failed: {e}")
    }
}

fn to_i32(v: u32) -> Result<i32, String> {
    i32::try_from(v).map_err(|_| "pointer out of range".to_owned())
}

/// `(ptr << 32) | len`, or `None` for a negative (error) value.
fn unpack(packed: i64) -> Option<(u32, u32)> {
    let v = u64::try_from(packed).ok()?;
    Some((
        u32::try_from(v >> 32).ok()?,
        u32::try_from(v & 0xffff_ffff).ok()?,
    ))
}

fn pack(ptr: u32, len: u32) -> i64 {
    // Memory is capped far below 2 GiB, so the pointer never reaches the sign bit.
    (i64::from(ptr) << 32) | i64::from(len)
}

fn span(p: i32, l: i32) -> Option<(usize, usize)> {
    let p = usize::try_from(p).ok()?;
    let l = usize::try_from(l).ok()?;
    (l <= MAX_IO).then_some((p, p.checked_add(l)?))
}

fn guest_str(c: &mut Caller<'_, State>, ptr: i32, len: i32) -> Option<String> {
    let mem = c.get_export("memory").and_then(Extern::into_memory)?;
    let (start, end) = span(ptr, len)?;
    let bytes = mem.data(&*c).get(start..end)?;
    String::from_utf8(bytes.to_vec()).ok()
}

/// Copy host data into guest memory through `kor_alloc`.
fn give(c: &mut Caller<'_, State>, data: &[u8]) -> i64 {
    let Some(alloc) = c
        .get_export("kor_alloc")
        .and_then(Extern::into_func)
        .and_then(|f| f.typed::<i32, i32>(&*c).ok())
    else {
        return FAILED;
    };
    let Ok(len) = i32::try_from(data.len()) else {
        return FAILED;
    };
    let Ok(ptr) = alloc.call(&mut *c, len) else {
        return FAILED;
    };
    let (Some(mem), Ok(off), Ok(len)) = (
        c.get_export("memory").and_then(Extern::into_memory),
        u32::try_from(ptr),
        u32::try_from(len),
    ) else {
        return FAILED;
    };
    match mem.write(&mut *c, off as usize, data) {
        Ok(()) => pack(off, len),
        Err(_) => FAILED,
    }
}

fn put(store: &mut Store<State>, inst: &Instance, data: &[u8]) -> Result<i64, String> {
    let alloc = inst
        .get_typed_func::<i32, i32>(&mut *store, "kor_alloc")
        .map_err(|e| format!("kor_alloc: {e}"))?;
    let len = i32::try_from(data.len()).map_err(|e| e.to_string())?;
    let ptr = alloc.call(&mut *store, len).map_err(|e| trap(store, &e))?;
    let off = u32::try_from(ptr).map_err(|_| "kor_alloc returned a negative pointer")?;
    let mem = inst
        .get_memory(&mut *store, "memory")
        .ok_or("the module exports no memory")?;
    mem.write(&mut *store, off as usize, data)
        .map_err(|e| e.to_string())?;
    Ok(pack(off, len.unsigned_abs()))
}

fn read_packed(store: &mut Store<State>, inst: &Instance, packed: i64) -> Result<Vec<u8>, String> {
    let (ptr, len) = unpack(packed).ok_or("the plugin returned an error")?;
    let mem = inst
        .get_memory(&mut *store, "memory")
        .ok_or("the module exports no memory")?;
    let (a, b) = span(to_i32(ptr)?, to_i32(len)?).ok_or("output is too large")?;
    mem.data(&*store)
        .get(a..b)
        .map(<[u8]>::to_vec)
        .ok_or_else(|| "output is outside the module's memory".into())
}

/// One entry of a plugin index (`kor plugin search`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexEntry {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Where to get it: Git URL, OCI reference or folder.
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub public_key: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum IndexFile {
    Wrapped { plugins: Vec<IndexEntry> },
    List(Vec<IndexEntry>),
}

/// Entries of a JSON index (`{"plugins": [...]}` or `[...]`) whose name, description or tags
/// contain every word of `query` (case-insensitive). An empty query lists all.
///
/// # Errors
/// If the index is not valid JSON of that shape.
pub fn search(index_json: &str, query: &str) -> Result<Vec<IndexEntry>, String> {
    let all = match serde_json::from_str::<IndexFile>(index_json).map_err(|e| e.to_string())? {
        IndexFile::Wrapped { plugins } | IndexFile::List(plugins) => plugins,
    };
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    Ok(all
        .into_iter()
        .filter(|e| {
            let hay = format!("{} {} {}", e.name, e.description, e.tags.join(" ")).to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .collect())
}

/// `kor plugin create`: a buildable plugin project in `dir` (`plugin.toml` + `plugin.wat`).
///
/// # Errors
/// If the name is unsafe, `dir` already exists, or a file cannot be written.
pub fn scaffold(dir: &Path, name: &str) -> Result<(), String> {
    if !plugin_name_ok(name) {
        return Err(format!("invalid plugin name `{name}`"));
    }
    if dir.exists() {
        return Err(format!("{} already exists", dir.display()));
    }
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let manifest = format!(
        "name = \"{name}\"\nversion = \"0.1.0\"\ndescription = \"\"\nmodule = \"plugin.wat\"\n\
         # Sign the module, then put the public key here (docs/plugins.md):\n\
         #   minisign -G -p {name}.pub -s {name}.key\n\
         #   minisign -S -s {name}.key -m plugin.wat\n\
         # public_key = \"RW...\"\n\
         # fs.read, event.emit, tool:<Name>; each one is shown to the user at install.\n\
         capabilities = [\"event.emit\"]\n\n[limits]\nfuel = 1000000000\nmemory_mb = 64\n"
    );
    fs::write(dir.join(MANIFEST), manifest).map_err(|e| e.to_string())?;
    fs::write(dir.join("plugin.wat"), SCAFFOLD_WAT).map_err(|e| e.to_string())?;
    Ok(())
}

const SCAFFOLD_WAT: &str = r#";; A Kor plugin (ABI: docs/plugins.md). Kor compiles this text itself; a binary .wasm works too.
(module
  (import "kor" "emit" (func $emit (param i32 i32) (result i64)))
  (memory (export "memory") 1)
  (data (i32.const 0) "plugin started")
  (global $next (mut i32) (i32.const 1024))
  ;; Bump allocator: Kor copies the input and host results here.
  (func (export "kor_alloc") (param $len i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $next))
    (global.set $next (i32.add (global.get $next) (local.get $len)))
    (if (i32.gt_u (global.get $next) (i32.mul (memory.size) (i32.const 65536)))
      (then (drop (memory.grow (i32.add (i32.shr_u (local.get $len) (i32.const 16)) (i32.const 1))))))
    (local.get $p))
  ;; kor_run(input) -> (ptr << 32) | len: sends one event and echoes the input.
  (func (export "kor_run") (param $p i32) (param $l i32) (result i64)
    (drop (call $emit (i32.const 0) (i32.const 14)))
    (i64.or
      (i64.shl (i64.extend_i32_u (local.get $p)) (i64.const 32))
      (i64.extend_i32_u (local.get $l)))))
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Uses every host import: reads the file named by the input, calls `tool:Grep` with it,
    /// emits the file's text, returns the tool's answer.
    const PROBE: &str = r#"(module
      (import "kor" "read_file" (func $read (param i32 i32) (result i64)))
      (import "kor" "call_tool" (func $tool (param i32 i32 i32 i32) (result i64)))
      (import "kor" "emit" (func $emit (param i32 i32) (result i64)))
      (memory (export "memory") 1)
      (data (i32.const 0) "Grep")
      (global $next (mut i32) (i32.const 1024))
      (func (export "kor_alloc") (param $len i32) (result i32)
        (local $p i32)
        (local.set $p (global.get $next))
        (global.set $next (i32.add (global.get $next) (local.get $len)))
        (local.get $p))
      (func $ptr (param $v i64) (result i32) (i32.wrap_i64 (i64.shr_u (local.get $v) (i64.const 32))))
      (func $len (param $v i64) (result i32) (i32.wrap_i64 (local.get $v)))
      (func (export "kor_run") (param $p i32) (param $l i32) (result i64)
        (local $file i64) (local $out i64)
        (local.set $file (call $read (local.get $p) (local.get $l)))
        (if (i64.lt_s (local.get $file) (i64.const 0)) (then (return (local.get $file))))
        (drop (call $emit (call $ptr (local.get $file)) (call $len (local.get $file))))
        (local.set $out (call $tool (i32.const 0) (i32.const 4) (local.get $p) (local.get $l)))
        (local.get $out)))"#;

    fn keypair() -> minisign::KeyPair {
        minisign::KeyPair::generate_unencrypted_keypair().unwrap()
    }

    /// A signed plugin directory from WAT text.
    fn signed(dir: &Path, name: &str, wat: &str, caps: &[&str], kp: &minisign::KeyPair) {
        fs::create_dir_all(dir).unwrap();
        let wasm = wat::parse_str(wat).unwrap();
        fs::write(dir.join("plugin.wasm"), &wasm).unwrap();
        let sig = minisign::sign(Some(&kp.pk), &kp.sk, &wasm[..], None, None).unwrap();
        fs::write(dir.join("plugin.wasm.minisig"), sig.to_string()).unwrap();
        let m = Manifest {
            name: name.into(),
            version: "1.0.0".into(),
            description: String::new(),
            module: default_module(),
            public_key: Some(kp.pk.to_base64()),
            capabilities: caps.iter().map(|c| (*c).to_owned()).collect(),
            limits: Limits {
                fuel: Some(5_000_000),
                memory_mb: Some(2),
            },
        };
        fs::write(dir.join(MANIFEST), toml::to_string(&m).unwrap()).unwrap();
    }

    fn yes(_: &Manifest) -> bool {
        true
    }

    fn probe_host(events: Rc<RefCell<Vec<String>>>) -> Host {
        Host {
            read_file: Box::new(|p| {
                if p == "a.txt" {
                    Ok(b"hello".to_vec())
                } else {
                    Err("missing".into())
                }
            }),
            call_tool: Box::new(|name, input| Ok(format!("{name}({input})"))),
            emit: Box::new(move |e| events.borrow_mut().push(e.to_owned())),
        }
    }

    #[test]
    fn signed_plugin_installs_runs_with_its_grants_and_revocation_takes_effect() {
        let t = tempfile::tempdir().unwrap();
        let (src, root) = (t.path().join("src"), t.path().join("root"));
        let kp = keypair();
        signed(
            &src,
            "probe",
            PROBE,
            &[FS_READ, EVENT_EMIT, "tool:Grep"],
            &kp,
        );
        let mut shown = Vec::new();
        let id = install(&root, &src, &mut |m| {
            shown.clone_from(&m.capabilities);
            true
        })
        .unwrap();
        assert_eq!(id, "probe");
        assert_eq!(shown, vec![FS_READ, EVENT_EMIT, "tool:Grep"]);
        let dir = root.join("probe");
        let m = read_manifest(&dir).unwrap();
        let caps = grants(&root)["probe"].capabilities.clone();

        let events = Rc::new(RefCell::new(Vec::new()));
        let out = run(&dir, &m, &caps, probe_host(Rc::clone(&events)), b"a.txt").unwrap();
        assert_eq!(out, b"Grep(a.txt)");
        assert_eq!(*events.borrow(), vec!["hello"]);

        // Without the tool grant the call is denied (-1 comes back as an error result).
        assert_eq!(revoke(&root, "probe", Some("tool:Grep")).unwrap().len(), 2);
        let caps = grants(&root)["probe"].capabilities.clone();
        let err = run(&dir, &m, &caps, probe_host(Rc::default()), b"a.txt").unwrap_err();
        assert!(err.contains("error"), "{err}");
        // Without fs.read the first host call is denied already; nothing is emitted.
        revoke(&root, "probe", None).unwrap();
        let events = Rc::new(RefCell::new(Vec::new()));
        assert!(run(&dir, &m, &[], probe_host(Rc::clone(&events)), b"a.txt").is_err());
        assert!(events.borrow().is_empty());
        forget(&root, "probe").unwrap();
        assert!(grants(&root).is_empty());
    }

    #[test]
    fn unsigned_tampered_or_rekeyed_plugins_are_refused() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("root");
        let kp = keypair();
        let src = t.path().join("p");
        signed(&src, "p", PROBE, &[], &kp);

        let mut m = read_manifest(&src).unwrap();
        m.public_key = None;
        fs::write(src.join(MANIFEST), toml::to_string(&m).unwrap()).unwrap();
        assert!(
            install(&root, &src, &mut yes)
                .unwrap_err()
                .contains("not signed")
        );

        signed(&src, "p", PROBE, &[], &kp);
        let mut wasm = fs::read(src.join("plugin.wasm")).unwrap();
        wasm.push(0);
        fs::write(src.join("plugin.wasm"), wasm).unwrap();
        assert!(
            install(&root, &src, &mut yes)
                .unwrap_err()
                .contains("verify")
        );

        signed(&src, "p", PROBE, &[], &kp);
        assert!(
            install(&root, &src, &mut |_| false)
                .unwrap_err()
                .contains("declined")
        );
        assert!(!root.join("p").exists());
        install(&root, &src, &mut yes).unwrap();
        signed(&src, "p", PROBE, &[], &keypair());
        assert!(
            install(&root, &src, &mut yes)
                .unwrap_err()
                .contains("different key")
        );

        let bad = t.path().join("bad");
        signed(&bad, "bad", PROBE, &["net.connect"], &kp);
        assert!(
            install(&root, &bad, &mut yes)
                .unwrap_err()
                .contains("unknown capability")
        );
    }

    #[test]
    fn fuel_and_memory_limits_stop_a_plugin() {
        let t = tempfile::tempdir().unwrap();
        let kp = keypair();
        let spin = t.path().join("spin");
        signed(
            &spin,
            "spin",
            r#"(module (memory (export "memory") 1)
                 (func (export "kor_run") (param i32 i32) (result i64) (loop (br 0)) (i64.const 0)))"#,
            &[],
            &kp,
        );
        let m = read_manifest(&spin).unwrap();
        let err = run(&spin, &m, &[], Host::denied(), b"").unwrap_err();
        assert!(err.contains("fuel"), "{err}");

        let grow = t.path().join("grow");
        // 2 MiB cap: growing by 64 pages (4 MiB) fails, so memory.grow returns -1.
        signed(
            &grow,
            "grow",
            r#"(module (memory (export "memory") 1)
                 (func (export "kor_run") (param i32 i32) (result i64)
                   (i64.extend_i32_s (memory.grow (i32.const 64)))))"#,
            &[],
            &kp,
        );
        let m = read_manifest(&grow).unwrap();
        assert!(
            run(&grow, &m, &[], Host::denied(), b"")
                .unwrap_err()
                .contains("error")
        );

        // Imports outside `kor` (e.g. WASI) do not link.
        let wasi = t.path().join("wasi");
        signed(
            &wasi,
            "wasi",
            r#"(module (import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32))))"#,
            &[],
            &kp,
        );
        let root = t.path().join("root");
        assert!(
            install(&root, &wasi, &mut yes)
                .unwrap_err()
                .contains("does not provide")
        );
    }

    /// §14 Faz 8 acceptance: a Claude Code skill folder installs unchanged as a WASM plugin, and
    /// the agent's skill loader sees the text the module returns.
    #[test]
    fn claude_code_skill_folder_loads_as_a_wasm_plugin() {
        let t = tempfile::tempdir().unwrap();
        let skill = t.path().join("ship-it");
        fs::create_dir_all(skill.join("reference")).unwrap();
        let text = "---\nname: ship-it\ndescription: Release checklist. Use when shipping.\n---\n\n# Ship it\n\nRun the tests, tag, push. Ünicode ✓\n";
        fs::write(skill.join("SKILL.md"), text).unwrap();
        fs::write(skill.join("reference").join("notes.md"), "notes").unwrap();
        let before = fs::read(skill.join("SKILL.md")).unwrap();

        let root = t.path().join("plugins");
        let mut asked = false;
        let id = install(&root, &skill, &mut |m| {
            asked = m.capabilities.is_empty();
            true
        })
        .unwrap();
        assert!(asked);
        assert_eq!(fs::read(skill.join("SKILL.md")).unwrap(), before);
        let dir = root.join(&id);
        let m = read_manifest(&dir).unwrap();
        assert_eq!(skill_text(&dir, &m).unwrap().as_deref(), Some(text));
        assert!(wasmparser_magic(
            &fs::read(dir.join("plugin.wasm")).unwrap()
        ));
        assert!(dir.join("skills/ship-it/reference/notes.md").is_file());

        let mut c = kor_compat::Compat::default();
        kor_compat::add_plugin(&mut c, &dir);
        let s = c
            .skills
            .iter()
            .find(|s| s.name == "ship-it")
            .expect("skill loaded");
        assert!(s.description.contains("Release checklist"), "{s:?}");
        assert!(
            crate::plugins::installed(&root)
                .iter()
                .any(|(i, _)| i == "ship-it")
        );
    }

    fn wasmparser_magic(b: &[u8]) -> bool {
        b.starts_with(b"\0asm")
    }

    #[test]
    fn scaffold_builds_and_runs_and_search_matches_words() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("hello");
        scaffold(&dir, "hello").unwrap();
        assert!(scaffold(&dir, "hello").is_err());
        let m = read_manifest(&dir).unwrap();
        let events = Rc::new(RefCell::new(Vec::new()));
        let ev = Rc::clone(&events);
        let host = Host {
            emit: Box::new(move |e| ev.borrow_mut().push(e.to_owned())),
            ..Host::denied()
        };
        let out = run(&dir, &m, &m.capabilities, host, b"ping").unwrap();
        assert_eq!(out, b"ping");
        assert_eq!(*events.borrow(), vec!["plugin started"]);

        let index = r#"{"plugins":[
          {"name":"rust-analyzer","description":"Rust LSP helpers","tags":["rust","lsp"]},
          {"name":"django","description":"Django rules","source":"https://example.invalid/django.git"}]}"#;
        let hits = search(index, "Rust analyzer").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "rust-analyzer");
        assert_eq!(search(index, "").unwrap().len(), 2);
        assert!(search("[]", "x").unwrap().is_empty());
        assert!(search("{", "x").is_err());
    }
}
