//! `kor chrome-native-host`: the browser starts this through the wrapper `kor chrome install`
//! registers; it relays the extension's native messages to the running Kor's bridge
//! (`kor_tools::chrome::native_host`, ADR 0058).

/// Start the Kor desktop app (the side panel's one-click start when Kor is not running).
fn launch() -> std::io::Result<()> {
    kor_platform::native_messaging::open("kor://running")
}

pub fn run_stdio() -> std::io::Result<()> {
    let args: Vec<String> = std::env::var("KOR_NATIVE_ORIGIN")
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let origin = kor_tools::chrome::native_host::origin_from_args(&args);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(kor_tools::chrome::native_host::relay(
        tokio::io::stdin(),
        tokio::io::stdout(),
        kor_tools::chrome::PORT,
        origin,
        launch,
    ))
}
