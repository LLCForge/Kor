//! `kor voice` (spec §10): microphone → local whisper.cpp → task text. No audio leaves the
//! machine: the recording is a temporary WAV file, transcribed by `whisper-cli` and deleted.
//!
//! whisper.cpp comes from `PATH` (`whisper-cli`, e.g. Homebrew `whisper-cpp` on macOS) or from the
//! verified components `whisper` (official v1.9.2 release builds: Windows x64, Linux x64/arm64) and
//! `whisper-model-base` (multilingual ggml-base, pinned Hugging Face revision). Downloads only
//! through the component flow, after the user agreed.
//!
//! Recording uses what the OS already has, no audio library is linked: Windows the built-in MCI
//! wave recorder (`winmm.dll`, through Windows `PowerShell`), macOS `AVAudioRecorder` (through
//! `osascript -l JavaScript`), Linux `arecord` (alsa-utils).

use kor_platform::NoWindow as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use kor_platform::Os;
use kor_providers::CancellationToken;
use serde::Serialize;

pub const WHISPER: &str = "whisper";
pub const WHISPER_MODEL: &str = "whisper-model-base";
pub const MAX_SECONDS: u32 = 120;

const WIN_SCRIPT: &str = r#"$ErrorActionPreference = 'Stop'
Add-Type -Namespace KorVoice -Name Mci -MemberDefinition '[DllImport("winmm.dll", CharSet = CharSet.Unicode)] public static extern int mciSendStringW(string c, System.Text.StringBuilder r, int n, System.IntPtr h); [DllImport("winmm.dll", CharSet = CharSet.Unicode)] public static extern bool mciGetErrorStringW(int e, System.Text.StringBuilder b, int n);'
function M($c) {
  $r = [KorVoice.Mci]::mciSendStringW($c, $null, 0, [IntPtr]::Zero)
  if ($r -ne 0) {
    $b = New-Object System.Text.StringBuilder 256
    [void][KorVoice.Mci]::mciGetErrorStringW($r, $b, 256)
    [void][KorVoice.Mci]::mciSendStringW('close korvoice', $null, 0, [IntPtr]::Zero)
    throw "microphone (MCI $r): $($b.ToString())"
  }
}
M 'open new type waveaudio alias korvoice'
M 'set korvoice format tag pcm bitspersample 16 samplespersec 16000 channels 1 alignment 2 bytespersec 32000'
M 'record korvoice'
Start-Sleep -Milliseconds ([int]$env:KOR_REC_MS)
M 'stop korvoice'
M ('save korvoice "' + $env:KOR_REC_PATH + '"')
M 'close korvoice'
"#;

const MAC_SCRIPT: &str = r"ObjC.import('Foundation'); ObjC.import('AVFoundation');
function run() {
  const env = $.NSProcessInfo.processInfo.environment;
  const path = ObjC.unwrap(env.objectForKey('KOR_REC_PATH'));
  const secs = parseFloat(ObjC.unwrap(env.objectForKey('KOR_REC_SECONDS')));
  const settings = $.NSDictionary.dictionaryWithObjectsForKeys(
    [$.NSNumber.numberWithInt(1819304813), $.NSNumber.numberWithFloat(16000), $.NSNumber.numberWithInt(1), $.NSNumber.numberWithInt(16)],
    [$('AVFormatIDKey'), $('AVSampleRateKey'), $('AVNumberOfChannelsKey'), $('AVLinearPCMBitDepthKey')]);
  const rec = $.AVAudioRecorder.alloc.initWithURLSettingsError($.NSURL.fileURLWithPath(path), settings, null);
  if (!rec || rec.isNil()) throw new Error('the microphone recorder could not start');
  if (!rec.recordForDuration(secs)) throw new Error('no microphone, or microphone access is not allowed for this terminal');
  $.NSRunLoop.currentRunLoop.runUntilDate($.NSDate.dateWithTimeIntervalSinceNow(secs + 0.3));
  rec.stop;
  return 'ok';
}";

/// Program, arguments and environment of a recorder.
pub type Recorder = (String, Vec<String>, Vec<(String, String)>);

/// The program and arguments that record `seconds` of 16 kHz mono PCM into `wav` on `os`.
pub fn recorder(os: Os, wav: &Path, seconds: u32) -> Result<Recorder, String> {
    let path = wav.to_string_lossy().into_owned();
    match os {
        Os::Windows => Ok((
            "powershell".into(),
            ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", WIN_SCRIPT]
                .map(str::to_owned)
                .to_vec(),
            vec![
                ("KOR_REC_PATH".into(), path),
                ("KOR_REC_MS".into(), (u64::from(seconds) * 1000).to_string()),
            ],
        )),
        Os::Mac => Ok((
            "osascript".into(),
            ["-l", "JavaScript", "-e", MAC_SCRIPT].map(str::to_owned).to_vec(),
            vec![
                ("KOR_REC_PATH".into(), path),
                ("KOR_REC_SECONDS".into(), seconds.to_string()),
            ],
        )),
        Os::Linux => Ok((
            "arecord".into(),
            vec![
                "-q".into(),
                "-f".into(),
                "S16_LE".into(),
                "-r".into(),
                "16000".into(),
                "-c".into(),
                "1".into(),
                "-d".into(),
                seconds.to_string(),
                path,
            ],
            Vec::new(),
        )),
        Os::Other => Err("recording is not available on this operating system".into()),
    }
}

async fn run(
    program: &str,
    args: &[String],
    env: &[(String, String)],
    timeout: Duration,
    cancel: &CancellationToken,
) -> Result<String, String> {
    let mut cmd = tokio::process::Command::new(program);
    cmd.no_window();
    cmd.args(args)
        .envs(env.iter().map(|(k, v)| (k, v)))
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let out = tokio::select! {
        r = tokio::time::timeout(timeout, cmd.output()) => match r {
            Err(_) => return Err(format!("{program} timed out")),
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(format!("`{program}` was not found on PATH"));
            }
            Ok(Err(e)) => return Err(format!("{program}: {e}")),
            Ok(Ok(o)) => o,
        },
        () = cancel.cancelled() => return Err("cancelled".into()),
    };
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("");
        Err(format!("{program} failed: {}", last.trim()))
    }
}

/// Record from the default microphone.
///
/// # Errors
/// No recorder or microphone, permission refused, timeout or cancellation.
pub async fn record(seconds: u32, wav: &Path, cancel: &CancellationToken) -> Result<(), String> {
    let seconds = seconds.clamp(1, MAX_SECONDS);
    let (program, args, env) = recorder(kor_platform::os(), wav, seconds)?;
    let timeout = Duration::from_secs(u64::from(seconds) + 30);
    run(&program, &args, &env, timeout, cancel).await?;
    match std::fs::metadata(wav) {
        // A WAV header alone is 44 bytes: nothing was recorded.
        Ok(m) if m.len() > 1024 => Ok(()),
        _ => Err("nothing was recorded (no microphone, or access was refused)".into()),
    }
}

fn on_path(name: &str) -> Option<PathBuf> {
    let exe = kor_platform::exe(name);
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(&exe))
        .find(|p| p.is_file())
}

/// What `kor voice` needs and what is missing.
#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub whisper: Option<PathBuf>,
    pub model: Option<PathBuf>,
    /// Components to download: name, version, size in bytes (`None`: no build for this machine).
    pub missing: Vec<(String, String, Option<u64>)>,
}

impl Status {
    pub fn ready(&self) -> bool {
        self.whisper.is_some() && self.model.is_some()
    }
}

/// `whisper-cli` on `PATH` or the installed component, and the installed model.
pub fn status() -> Status {
    let whisper = on_path("whisper-cli").or_else(|| kor_components::path(WHISPER));
    let model = kor_components::path(WHISPER_MODEL);
    let mut missing = Vec::new();
    for (name, have) in [(WHISPER, whisper.is_some()), (WHISPER_MODEL, model.is_some())] {
        if !have && let Some(s) = kor_components::info(name) {
            missing.push((s.name, s.version, s.size));
        }
    }
    Status {
        whisper,
        model,
        missing,
    }
}

/// `whisper-cli` arguments: no timestamps, no progress, language auto-detected unless given.
pub fn whisper_args(model: &Path, wav: &Path, language: Option<&str>) -> Vec<String> {
    let lang = language
        .filter(|l| !l.is_empty() && l.len() <= 8 && l.chars().all(|c| c.is_ascii_alphabetic()))
        .unwrap_or("auto");
    vec![
        "-m".into(),
        model.to_string_lossy().into_owned(),
        "-f".into(),
        wav.to_string_lossy().into_owned(),
        "-nt".into(),
        "-np".into(),
        "-l".into(),
        lang.to_ascii_lowercase(),
    ]
}

/// whisper prints one segment per line; blank-audio markers are dropped.
pub fn clean(stdout: &str) -> String {
    stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("[BLANK_AUDIO]") && *l != "[ Silence ]")
        .collect::<Vec<_>>()
        .join(" ")
}

/// Transcribe a 16 kHz WAV locally.
///
/// # Errors
/// whisper failing or printing nothing.
pub async fn transcribe(
    whisper: &Path,
    model: &Path,
    wav: &Path,
    language: Option<&str>,
    cancel: &CancellationToken,
) -> Result<String, String> {
    let out = run(
        &whisper.to_string_lossy(),
        &whisper_args(model, wav, language),
        &[],
        Duration::from_secs(600),
        cancel,
    )
    .await?;
    let text = clean(&out);
    if text.is_empty() {
        Err("no speech was recognised".into())
    } else {
        Ok(text)
    }
}

/// Record, transcribe, delete the recording. Needs [`status`] to be ready.
///
/// # Errors
/// whisper or its model missing, or any recording/transcription error.
pub async fn listen(seconds: u32, language: Option<&str>, cancel: &CancellationToken) -> Result<String, String> {
    let st = status();
    let (Some(whisper), Some(model)) = (st.whisper, st.model) else {
        return Err(format!(
            "whisper.cpp is not installed: kor component add {WHISPER} && kor component add {WHISPER_MODEL}"
        ));
    };
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let wav = std::env::temp_dir().join(format!("kor-voice-{}-{nanos}.wav", std::process::id()));
    let result = async {
        record(seconds, &wav, cancel).await?;
        transcribe(&whisper, &model, &wav, language, cancel).await
    }
    .await;
    let _ = std::fs::remove_file(&wav);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorders_per_os() {
        let wav = Path::new("/tmp/a.wav");
        let (p, a, e) = recorder(Os::Linux, wav, 5).unwrap();
        assert_eq!(p, "arecord");
        assert!(a.ends_with(&["5".into(), "/tmp/a.wav".into()]) && e.is_empty());
        let (p, a, e) = recorder(Os::Windows, wav, 3).unwrap();
        assert_eq!(p, "powershell");
        assert!(a.last().unwrap().contains("mciSendStringW") && a.last().unwrap().contains("samplespersec 16000"));
        assert!(e.contains(&("KOR_REC_MS".into(), "3000".into())));
        let (p, a, _) = recorder(Os::Mac, wav, 3).unwrap();
        assert_eq!(p, "osascript");
        assert!(a[3].contains("AVAudioRecorder") && a[3].contains("function run()"));
        assert!(recorder(Os::Other, wav, 1).is_err());
    }

    #[test]
    fn whisper_arguments_and_output() {
        let a = whisper_args(Path::new("m.bin"), Path::new("a.wav"), Some("TR"));
        assert_eq!(a, ["-m", "m.bin", "-f", "a.wav", "-nt", "-np", "-l", "tr"]);
        let a = whisper_args(Path::new("m.bin"), Path::new("a.wav"), Some("tr; rm -rf"));
        assert_eq!(a.last().map(String::as_str), Some("auto"));
        assert_eq!(clean(" Add a login button.\n\n[BLANK_AUDIO]\n to settings\n"), "Add a login button. to settings");
        assert_eq!(clean("[BLANK_AUDIO]\n"), "");
    }

    #[test]
    fn components_are_pinned() {
        let m = kor_components::Manifest::builtin();
        let w = m.get(WHISPER).expect("whisper component");
        assert!(w.artifact("windows-x86_64").is_some() && w.artifact("linux-x86_64").is_some());
        let model = m.get(WHISPER_MODEL).expect("model component");
        let a = model.artifact("any").expect("model is target-independent");
        assert_eq!(a.size, 147_951_465);
        assert_eq!(a.sha256, "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe");
    }

    /// `cargo test -p kor-runtime -- --ignored records_from`
    #[tokio::test]
    #[ignore = "needs a microphone (and, on macOS, microphone access for the terminal)"]
    async fn records_from_the_microphone() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("m.wav");
        record(2, &wav, &CancellationToken::new()).await.unwrap();
        assert!(std::fs::metadata(&wav).unwrap().len() > 30_000);
    }

    /// Real download (~156 MB) and a transcription of speech synthesized by Windows.
    /// `cargo test -p kor-runtime -- --ignored whisper_transcribes`
    #[tokio::test]
    #[ignore = "downloads whisper.cpp and the ggml-base model; needs Windows speech synthesis"]
    async fn whisper_transcribes_synthesized_speech() {
        if kor_platform::os() != Os::Windows {
            return;
        }
        let quiet = |_: kor_components::Progress| {};
        let dir = tempfile::tempdir().unwrap();
        // A throwaway store: the real download and checksum path, nothing left in the user's data.
        let store = kor_components::Store::new(dir.path().join("c"), kor_components::Manifest::builtin());
        let target = kor_components::target();
        let exe = store.ensure_for(WHISPER, &target, &quiet).await.unwrap();
        let model = store.ensure_for(WHISPER_MODEL, &target, &quiet).await.unwrap();
        let wav = dir.path().join("t.wav");
        let say = "Add-Type -AssemblyName System.Speech; $s=New-Object System.Speech.Synthesis.SpeechSynthesizer; $f=New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(16000,[System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen,[System.Speech.AudioFormat.AudioChannel]::Mono); $s.SetOutputToWaveFile($env:P,$f); $s.Speak('Add a login button to the settings page.'); $s.Dispose()";
        let ok = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", say])
            .env("P", &wav)
            .status()
            .unwrap();
        assert!(ok.success());
        let text = transcribe(&exe, &model, &wav, None, &CancellationToken::new())
            .await
            .unwrap();
        assert!(text.to_lowercase().contains("login button"), "{text}");
    }
}
