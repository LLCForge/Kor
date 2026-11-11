//! Win32 side of [`super`]. The only `unsafe` code in the backend workspace (ADR 0008): every
//! block is one FFI call with pointers to locals that outlive it, and every returned handle or
//! allocation is wrapped in an owner that frees it.
#![allow(unsafe_code)]

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::io::{self, Read as _};
use std::os::windows::ffi::{OsStrExt as _, OsStringExt as _};
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
use std::os::windows::process::ExitStatusExt as _;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Output};
use std::ptr::{null, null_mut};
use std::sync::{LazyLock, Mutex, OnceLock, PoisonError};

use windows_sys::Win32::Foundation::{
    HANDLE, HANDLE_FLAG_INHERIT, LocalFree, SetHandleInformation, WAIT_OBJECT_0,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSidToSidW, EXPLICIT_ACCESS_W, GRANT_ACCESS,
    GetNamedSecurityInfoW, NO_MULTIPLE_TRUSTEE, REVOKE_ACCESS, SE_FILE_OBJECT, SetEntriesInAclW,
    SetNamedSecurityInfoW, TRUSTEE_IS_SID, TRUSTEE_IS_WELL_KNOWN_GROUP, TRUSTEE_W,
};
use windows_sys::Win32::Security::Isolation::{
    CreateAppContainerProfile, DeleteAppContainerProfile,
    DeriveAppContainerSidFromAppContainerName, GetAppContainerFolderPath,
};
use windows_sys::Win32::Security::{
    ACL, DACL_SECURITY_INFORMATION, FreeSid, PSECURITY_DESCRIPTOR, PSID, SECURITY_CAPABILITIES,
    SID_AND_ATTRIBUTES, SUB_CONTAINERS_AND_OBJECTS_INHERIT,
};
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_CPU_RATE_CONTROL_ENABLE,
    JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP, JOB_OBJECT_LIMIT_JOB_MEMORY,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_CPU_RATE_CONTROL_INFORMATION,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectCpuRateControlInformation,
    JobObjectExtendedLimitInformation, SetInformationJobObject,
};
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
    DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, INFINITE,
    InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
    PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOEXW, TerminateProcess,
    UpdateProcThreadAttribute, WaitForSingleObject,
};
use windows_sys::core::{BOOL, HRESULT, PWSTR};

use super::Spec;

const FILE_ALL_ACCESS: u32 = 0x001F_01FF;
const SE_GROUP_ENABLED: u32 = 0x4;
/// `HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS)`.
const HR_ALREADY_EXISTS: HRESULT = 0x8007_00B7_u32.cast_signed();
/// `internetClient` and `privateNetworkClientServer`.
const NETWORK_CAPABILITIES: [&str; 2] = ["S-1-15-3-1", "S-1-15-3-3"];

// ---------- errors and owners ----------

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, what.to_owned())
}

fn check(b: BOOL) -> io::Result<()> {
    if b == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn check_win32(code: u32) -> io::Result<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(code.cast_signed()))
    }
}

fn check_hr(hr: HRESULT) -> io::Result<()> {
    let u = hr.cast_unsigned();
    if hr >= 0 {
        Ok(())
    } else if u & 0xFFFF_0000 == 0x8007_0000 {
        Err(io::Error::from_raw_os_error((u & 0xFFFF).cast_signed()))
    } else {
        Err(io::Error::other(format!("HRESULT {u:#010x}")))
    }
}

fn context(e: &io::Error, what: &str) -> io::Error {
    io::Error::new(e.kind(), format!("{what}: {e}"))
}

fn size32<T>() -> u32 {
    u32::try_from(size_of::<T>()).unwrap_or(u32::MAX)
}

/// NUL-terminated UTF-16; refuses embedded NULs (they would silently cut the string).
fn wide(s: &OsStr) -> io::Result<Vec<u16>> {
    let mut v: Vec<u16> = s.encode_wide().collect();
    if v.contains(&0) {
        return Err(invalid("argument contains a NUL character"));
    }
    v.push(0);
    Ok(v)
}

/// Memory from `LocalAlloc` (security descriptors, ACLs, string SIDs).
struct Local(*mut core::ffi::c_void);

impl Drop for Local {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the pointer came from an API documented to allocate with LocalAlloc.
            unsafe { LocalFree(self.0) };
        }
    }
}

/// A SID allocated by userenv (`FreeSid`) or by `ConvertStringSidToSidW` (`LocalFree`).
struct Sid {
    ptr: PSID,
    local: bool,
}

impl Drop for Sid {
    fn drop(&mut self) {
        // SAFETY: freed exactly once, with the deallocator its API documents.
        unsafe {
            if self.local {
                LocalFree(self.ptr);
            } else {
                FreeSid(self.ptr);
            }
        }
    }
}

fn owned(h: HANDLE) -> io::Result<OwnedHandle> {
    if h.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a fresh handle that nothing else owns.
    Ok(unsafe { OwnedHandle::from_raw_handle(h) })
}

// ---------- profiles and ACL grants ----------

fn container_sid(name: &str) -> io::Result<Sid> {
    let w = wide(OsStr::new(name))?;
    let mut ptr = null_mut();
    // SAFETY: `w` is NUL-terminated and outlives the call; `ptr` receives a SID we own.
    check_hr(unsafe { DeriveAppContainerSidFromAppContainerName(w.as_ptr(), &raw mut ptr) })?;
    Ok(Sid { ptr, local: false })
}

fn create_profile(name: &str) -> io::Result<()> {
    let w = wide(OsStr::new(name))?;
    let desc = wide(OsStr::new("Kor agent command sandbox"))?;
    let mut ptr = null_mut();
    // SAFETY: strings outlive the call; no capabilities; `ptr` receives a SID we own.
    let hr = unsafe {
        CreateAppContainerProfile(
            w.as_ptr(),
            w.as_ptr(),
            desc.as_ptr(),
            null(),
            0,
            &raw mut ptr,
        )
    };
    if hr == HR_ALREADY_EXISTS {
        return Ok(());
    }
    check_hr(hr)?;
    drop(Sid { ptr, local: false });
    Ok(())
}

fn delete_profile(name: &str) {
    if let Ok(w) = wide(OsStr::new(name)) {
        // SAFETY: NUL-terminated string that outlives the call.
        let _ = unsafe { DeleteAppContainerProfile(w.as_ptr()) };
    }
}

/// Add (or remove every) full-control inheritable ACE for `sid` on `root` and its tree.
fn set_acl(root: &Path, sid: &Sid, grant: bool) -> io::Result<()> {
    let path = wide(root.as_os_str())?;
    let mut dacl: *mut ACL = null_mut();
    let mut sd: PSECURITY_DESCRIPTOR = null_mut();
    // SAFETY: out-pointers to locals; `dacl` points into `sd`, freed by `_sd` after its last use.
    check_win32(unsafe {
        GetNamedSecurityInfoW(
            path.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            &raw mut dacl,
            null_mut(),
            &raw mut sd,
        )
    })?;
    let _sd = Local(sd);
    let access = EXPLICIT_ACCESS_W {
        grfAccessPermissions: if grant { FILE_ALL_ACCESS } else { 0 },
        grfAccessMode: if grant { GRANT_ACCESS } else { REVOKE_ACCESS },
        grfInheritance: SUB_CONTAINERS_AND_OBJECTS_INHERIT,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: null_mut(),
            MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_WELL_KNOWN_GROUP,
            ptstrName: sid.ptr.cast(),
        },
    };
    let mut new_acl: *mut ACL = null_mut();
    // SAFETY: one entry read from a local; the new ACL is ours to LocalFree.
    check_win32(unsafe { SetEntriesInAclW(1, &raw const access, dacl, &raw mut new_acl) })?;
    let _new = Local(new_acl.cast());
    // SAFETY: path and ACL outlive the call. Windows propagates the inheritable ACE to the tree.
    check_win32(unsafe {
        SetNamedSecurityInfoW(
            path.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            new_acl,
            null(),
        )
    })
}

/// Live confined commands per profile; the grant exists while the count is above zero.
static GRANTS: LazyLock<Mutex<HashMap<String, usize>>> = LazyLock::new(Mutex::default);

/// The project root's profile and ACL grant, shared by concurrent commands of that root and
/// removed (ACE revoked, profile and its temp folder deleted) when the last one ends.
#[derive(Debug)]
struct Grant {
    root: PathBuf,
    name: String,
}

impl Grant {
    fn acquire(root: &Path) -> io::Result<Self> {
        let name = super::profile_name(root);
        let mut grants = GRANTS.lock().unwrap_or_else(PoisonError::into_inner);
        let count = grants.entry(name.clone()).or_insert(0);
        if *count == 0 {
            create_profile(&name)?;
            if let Err(e) = container_sid(&name).and_then(|sid| set_acl(root, &sid, true)) {
                delete_profile(&name);
                return Err(e);
            }
        }
        *count += 1;
        Ok(Self {
            root: root.to_path_buf(),
            name,
        })
    }
}

impl Drop for Grant {
    fn drop(&mut self) {
        let mut grants = GRANTS.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(count) = grants.get_mut(&self.name) else {
            return;
        };
        *count -= 1;
        if *count == 0 {
            grants.remove(&self.name);
            if let Ok(sid) = container_sid(&self.name) {
                let _ = set_acl(&self.root, &sid, false);
            }
            delete_profile(&self.name);
        }
    }
}

/// `%LOCALAPPDATA%\Packages\<profile>\AC\Temp`: the container's own temp folder.
fn temp_dir(sid: &Sid) -> io::Result<PathBuf> {
    let mut text: PWSTR = null_mut();
    // SAFETY: valid SID; the string is ours to LocalFree.
    check(unsafe { ConvertSidToStringSidW(sid.ptr, &raw mut text) })?;
    let _text = Local(text.cast());
    let mut folder: PWSTR = null_mut();
    // SAFETY: NUL-terminated SID string; the path is ours to CoTaskMemFree.
    check_hr(unsafe { GetAppContainerFolderPath(text, &raw mut folder) })?;
    // SAFETY: `folder` is a NUL-terminated string from the call above, freed right after copying.
    let path = unsafe {
        let mut len = 0;
        while *folder.add(len) != 0 {
            len += 1;
        }
        let s = OsString::from_wide(std::slice::from_raw_parts(folder, len));
        CoTaskMemFree(folder.cast());
        PathBuf::from(s)
    };
    let temp = path.join("Temp");
    std::fs::create_dir_all(&temp)?;
    Ok(temp)
}

fn string_sid(s: &str) -> io::Result<Sid> {
    let w = wide(OsStr::new(s))?;
    let mut ptr = null_mut();
    // SAFETY: NUL-terminated string; the SID is ours to LocalFree.
    check(unsafe { ConvertStringSidToSidW(w.as_ptr(), &raw mut ptr) })?;
    Ok(Sid { ptr, local: true })
}

pub(super) fn available() -> bool {
    static PROBE: OnceLock<bool> = OnceLock::new();
    // Deriving a SID creates nothing; it fails where AppContainers do not exist.
    *PROBE.get_or_init(|| container_sid("kor.sandbox.probe").is_ok())
}

// ---------- command line, environment, program lookup ----------

const QUOTE: u16 = b'"' as u16;
const BACKSLASH: u16 = b'\\' as u16;

/// Append `arg` quoted the way `CommandLineToArgvW` and the MSVC runtime parse it back.
fn quote(arg: &OsStr, out: &mut Vec<u16>) -> io::Result<()> {
    let w: Vec<u16> = arg.encode_wide().collect();
    if w.contains(&0) {
        return Err(invalid("argument contains a NUL character"));
    }
    let special =
        |c: &u16| [b' ', b'\t', b'\n', 0x0b, b'"'].contains(&u8::try_from(*c).unwrap_or(0));
    if !w.is_empty() && !w.iter().any(special) {
        out.extend(w);
        return Ok(());
    }
    out.push(QUOTE);
    let mut backslashes = 0;
    for c in w {
        if c == BACKSLASH {
            backslashes += 1;
            continue;
        }
        let n = if c == QUOTE {
            backslashes * 2 + 1
        } else {
            backslashes
        };
        out.extend(std::iter::repeat_n(BACKSLASH, n));
        out.push(c);
        backslashes = 0;
    }
    out.extend(std::iter::repeat_n(BACKSLASH, backslashes * 2));
    out.push(QUOTE);
    Ok(())
}

fn command_line(program: &OsStr, args: &[OsString]) -> io::Result<Vec<u16>> {
    let mut out = Vec::new();
    for (i, a) in std::iter::once(program)
        .chain(args.iter().map(OsString::as_os_str))
        .enumerate()
    {
        if i > 0 {
            out.push(u16::from(b' '));
        }
        quote(a, &mut out)?;
    }
    out.push(0);
    Ok(out)
}

/// `KEY=VALUE\0...\0\0`, sorted, with `TEMP`/`TMP` pointing at the container's temp folder.
fn env_block(env: &[(OsString, OsString)], temp: &Path) -> io::Result<Vec<u16>> {
    let is_temp = |k: &OsStr| k.eq_ignore_ascii_case("TEMP") || k.eq_ignore_ascii_case("TMP");
    let mut vars: Vec<(&OsStr, &OsStr)> = env
        .iter()
        .filter(|(k, _)| !is_temp(k))
        .map(|(k, v)| (k.as_os_str(), v.as_os_str()))
        .collect();
    vars.push((OsStr::new("TEMP"), temp.as_os_str()));
    vars.push((OsStr::new("TMP"), temp.as_os_str()));
    vars.sort_by_cached_key(|(k, _)| k.to_ascii_uppercase());
    let mut out = Vec::new();
    for (k, v) in vars {
        let k: Vec<u16> = k.encode_wide().collect();
        if k.is_empty() || k[1..].contains(&u16::from(b'=')) {
            continue;
        }
        let v: Vec<u16> = v.encode_wide().collect();
        if k.contains(&0) || v.contains(&0) {
            return Err(invalid("environment contains a NUL character"));
        }
        out.extend(k);
        out.push(u16::from(b'='));
        out.extend(v);
        out.push(0);
    }
    if out.is_empty() {
        out.push(0);
    }
    out.push(0);
    Ok(out)
}

/// Full path of `program`: as given when it has a directory, else the first `PATH` hit (adding
/// `.exe`/`.com` when it has no extension). Never the current directory.
fn resolve(program: &OsStr, env: &[(OsString, OsString)]) -> io::Result<PathBuf> {
    let p = Path::new(program);
    if p.components().count() > 1 {
        return Ok(p.to_path_buf());
    }
    let path = env
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("PATH"))
        .map(|(_, v)| v.clone())
        .or_else(|| std::env::var_os("PATH"))
        .unwrap_or_default();
    let names: Vec<OsString> = if p.extension().is_some() {
        vec![program.to_owned()]
    } else {
        [".exe", ".com"]
            .iter()
            .map(|e| {
                let mut n = program.to_owned();
                n.push(e);
                n
            })
            .collect()
    };
    std::env::split_paths(&path)
        .flat_map(|d| names.iter().map(move |n| d.join(n)))
        .find(|c| c.is_file())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("{} was not found on PATH", program.to_string_lossy()),
            )
        })
}

/// `\\?\C:\x` to `C:\x`: `PowerShell` cannot start in a verbatim directory.
fn without_verbatim(p: &Path) -> &Path {
    match p.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => Path::new(rest),
        _ => p,
    }
}

// ---------- process ----------

/// `PROC_THREAD_ATTRIBUTE_LIST` in a pointer-aligned buffer.
struct AttrList {
    buf: Vec<usize>,
}

impl AttrList {
    fn new(count: u32) -> io::Result<Self> {
        let mut size = 0;
        // SAFETY: size query with a null list; expected to fail with ERROR_INSUFFICIENT_BUFFER.
        let _ = unsafe { InitializeProcThreadAttributeList(null_mut(), count, 0, &raw mut size) };
        let mut buf = vec![0usize; size.div_ceil(size_of::<usize>())];
        // SAFETY: `buf` is at least `size` bytes and pointer-aligned.
        check(unsafe {
            InitializeProcThreadAttributeList(buf.as_mut_ptr().cast(), count, 0, &raw mut size)
        })?;
        Ok(Self { buf })
    }

    fn ptr(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.buf.as_mut_ptr().cast()
    }

    /// # Safety
    /// `value` must stay valid until the process is created.
    unsafe fn set<T>(&mut self, attribute: u32, value: *const T, size: usize) -> io::Result<()> {
        // SAFETY: initialised list; caller keeps `value` alive.
        check(unsafe {
            UpdateProcThreadAttribute(
                self.ptr(),
                0,
                attribute as usize,
                value.cast(),
                size,
                null_mut(),
                null(),
            )
        })
    }
}

impl Drop for AttrList {
    fn drop(&mut self) {
        // SAFETY: initialised in `new`.
        unsafe { DeleteProcThreadAttributeList(self.ptr()) };
    }
}

#[derive(Debug)]
pub(super) struct Child {
    // Drop order matters: the job first (kills the tree), the grant last.
    _job: OwnedHandle,
    process: OwnedHandle,
    stdout: std::io::PipeReader,
    stderr: std::io::PipeReader,
    pub(super) pid: u32,
    _grant: Grant,
}

fn job_for(process: &OwnedHandle, spec: &Spec<'_>) -> io::Result<OwnedHandle> {
    // SAFETY: no attributes, no name.
    let job = owned(unsafe { CreateJobObjectW(null(), null()) })?;
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags =
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_JOB_MEMORY;
    limits.JobMemoryLimit = usize::try_from(spec.memory_bytes).unwrap_or(usize::MAX);
    // SAFETY: a local struct of the size passed.
    check(unsafe {
        SetInformationJobObject(
            job.as_raw_handle(),
            JobObjectExtendedLimitInformation,
            (&raw const limits).cast(),
            size32::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>(),
        )
    })?;
    if (1..100).contains(&spec.cpu_percent) {
        let mut cpu = JOBOBJECT_CPU_RATE_CONTROL_INFORMATION {
            ControlFlags: JOB_OBJECT_CPU_RATE_CONTROL_ENABLE | JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP,
            ..Default::default()
        };
        // Units of 1/100 percent.
        cpu.Anonymous.CpuRate = u32::from(spec.cpu_percent) * 100;
        // SAFETY: a local struct of the size passed.
        check(unsafe {
            SetInformationJobObject(
                job.as_raw_handle(),
                JobObjectCpuRateControlInformation,
                (&raw const cpu).cast(),
                size32::<JOBOBJECT_CPU_RATE_CONTROL_INFORMATION>(),
            )
        })?;
    }
    // SAFETY: both handles are live.
    check(unsafe { AssignProcessToJobObject(job.as_raw_handle(), process.as_raw_handle()) })?;
    Ok(job)
}

fn capabilities(network: bool) -> io::Result<Vec<Sid>> {
    let names: &[&str] = if network { &NETWORK_CAPABILITIES } else { &[] };
    names
        .iter()
        .map(|s| string_sid(s))
        .collect::<io::Result<_>>()
        .map_err(|e| context(&e, "capability SID"))
}

pub(super) fn spawn(spec: &Spec<'_>) -> io::Result<Child> {
    let program = resolve(spec.program, spec.env)?;
    let grant = Grant::acquire(spec.root)?;
    let sid = container_sid(&grant.name)?;
    let temp = temp_dir(&sid)?;

    let caps = capabilities(spec.network)?;
    let mut cap_attrs: Vec<SID_AND_ATTRIBUTES> = caps
        .iter()
        .map(|c| SID_AND_ATTRIBUTES {
            Sid: c.ptr,
            Attributes: SE_GROUP_ENABLED,
        })
        .collect();
    let security = SECURITY_CAPABILITIES {
        AppContainerSid: sid.ptr,
        Capabilities: if cap_attrs.is_empty() {
            null_mut()
        } else {
            cap_attrs.as_mut_ptr()
        },
        CapabilityCount: u32::try_from(cap_attrs.len()).unwrap_or(0),
        Reserved: 0,
    };

    // stdin: a pipe whose write end is closed at once (immediate EOF).
    let (stdin_r, stdin_w) = std::io::pipe()?;
    drop(stdin_w);
    let (stdout_r, stdout_w) = std::io::pipe()?;
    let (stderr_r, stderr_w) = std::io::pipe()?;
    let inherited: [HANDLE; 3] = [
        stdin_r.as_raw_handle(),
        stdout_w.as_raw_handle(),
        stderr_w.as_raw_handle(),
    ];
    // shortcut: these three are inheritable until CreateProcess returns, so a std::process spawn on
    // another thread in that window may inherit them too (its child then holds our pipes open).
    // The HANDLE_LIST below keeps *our* child from inheriting anything else.
    for h in inherited {
        // SAFETY: live pipe handles owned by locals above.
        check(unsafe { SetHandleInformation(h, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) })?;
    }

    let mut attrs = AttrList::new(2)?;
    // SAFETY: `security` (and the SIDs it points to) and `inherited` live until after CreateProcessW.
    unsafe {
        attrs.set(
            PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
            &raw const security,
            size_of::<SECURITY_CAPABILITIES>(),
        )?;
        attrs.set(
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
            inherited.as_ptr(),
            size_of_val(&inherited),
        )?;
    }
    let mut si = STARTUPINFOEXW::default();
    si.StartupInfo.cb = size32::<STARTUPINFOEXW>();
    si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    si.StartupInfo.hStdInput = inherited[0];
    si.StartupInfo.hStdOutput = inherited[1];
    si.StartupInfo.hStdError = inherited[2];
    si.lpAttributeList = attrs.ptr();

    let app = wide(program.as_os_str())?;
    let mut cmdline = command_line(spec.program, spec.args)?;
    let env = env_block(spec.env, &temp)?;
    let cwd = wide(without_verbatim(spec.cwd).as_os_str())?;
    let mut pi = PROCESS_INFORMATION::default();
    // SAFETY: every pointer is to a local that outlives the call; the returned process and
    // thread handles are wrapped right after.
    check(unsafe {
        CreateProcessW(
            app.as_ptr(),
            cmdline.as_mut_ptr(),
            null(),
            null(),
            1,
            EXTENDED_STARTUPINFO_PRESENT
                | CREATE_SUSPENDED
                | CREATE_UNICODE_ENVIRONMENT
                | CREATE_NO_WINDOW,
            env.as_ptr().cast(),
            cwd.as_ptr(),
            (&raw const si).cast(),
            &raw mut pi,
        )
    })
    .map_err(|e| context(&e, "CreateProcessW"))?;
    let process = owned(pi.hProcess)?;
    let thread = owned(pi.hThread)?;
    drop((stdin_r, stdout_w, stderr_w, attrs));

    let job = match job_for(&process, spec) {
        Ok(j) => j,
        Err(e) => {
            // SAFETY: live process handle; it is still suspended.
            unsafe { TerminateProcess(process.as_raw_handle(), 1) };
            return Err(e);
        }
    };
    // SAFETY: live thread handle. On failure dropping `job` kills the suspended process.
    if unsafe { ResumeThread(thread.as_raw_handle()) } == u32::MAX {
        return Err(io::Error::last_os_error());
    }
    Ok(Child {
        _job: job,
        process,
        stdout: stdout_r,
        stderr: stderr_r,
        pid: pi.dwProcessId,
        _grant: grant,
    })
}

impl Child {
    pub(super) async fn wait_with_output(self) -> io::Result<Output> {
        let process = self.process.try_clone()?;
        let mut stdout = self.stdout.try_clone()?;
        let mut stderr = self.stderr.try_clone()?;
        // `self` (job and grant) stays in this future: dropping the future kills the tree, which
        // closes the pipes and lets the blocking reader below finish.
        let out = tokio::task::spawn_blocking(move || -> io::Result<Output> {
            let err = std::thread::spawn(move || {
                let mut v = Vec::new();
                stderr.read_to_end(&mut v).map(|_| v)
            });
            let mut out = Vec::new();
            stdout.read_to_end(&mut out)?;
            let err = err
                .join()
                .map_err(|_| io::Error::other("stderr reader failed"))??;
            // SAFETY: live process handle.
            if unsafe { WaitForSingleObject(process.as_raw_handle(), INFINITE) } != WAIT_OBJECT_0 {
                return Err(io::Error::last_os_error());
            }
            let mut code = 0u32;
            // SAFETY: live process handle; out-pointer to a local.
            check(unsafe { GetExitCodeProcess(process.as_raw_handle(), &raw mut code) })?;
            Ok(Output {
                status: ExitStatus::from_raw(code),
                stdout: out,
                stderr: err,
            })
        })
        .await
        .map_err(io::Error::other)?;
        drop(self);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(program: &str, args: &[&str]) -> String {
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        let w = command_line(OsStr::new(program), &args).unwrap();
        String::from_utf16(&w[..w.len() - 1]).unwrap()
    }

    #[test]
    fn command_line_quotes_like_msvc() {
        assert_eq!(
            line("pwsh", &["-Command", "echo hi"]),
            r#"pwsh -Command "echo hi""#
        );
        assert_eq!(line("p", &[""]), r#"p """#);
        assert_eq!(line("p", &[r#"a"b"#]), r#"p "a\"b""#);
        assert_eq!(line("p", &[r"a\\b c\"]), r#"p "a\\b c\\""#);
        assert_eq!(line("p", &[r#"x\"y z"#]), r#"p "x\\\"y z""#);
        assert!(command_line(OsStr::new("p"), &[OsString::from("a\0b")]).is_err());
    }

    #[test]
    fn verbatim_disk_paths_are_simplified() {
        assert_eq!(
            without_verbatim(Path::new(r"\\?\C:\a b")),
            Path::new(r"C:\a b")
        );
        let unc = Path::new(r"\\?\UNC\srv\x");
        assert_eq!(without_verbatim(unc), unc);
    }

    #[test]
    fn env_block_replaces_temp_and_terminates() {
        let env = vec![
            (OsString::from("b"), OsString::from("2")),
            (
                OsString::from("TEMP"),
                OsString::from(r"C:\Users\user\AppData\Local\Temp"),
            ),
            (OsString::from("A"), OsString::from("1")),
        ];
        let block = env_block(&env, Path::new(r"C:\t")).unwrap();
        let text = String::from_utf16(&block).unwrap();
        assert_eq!(text, "A=1\0b=2\0TEMP=C:\\t\0TMP=C:\\t\0\0");
    }
}
