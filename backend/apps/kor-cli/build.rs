//! `cfg(kor_proxy)`: the `proxy` feature is on and the target can build kor-proxy (BoringSSL).
//! Keep the target list in step with the target-specific `kor-proxy` dependency in Cargo.toml.

fn main() {
    println!("cargo::rustc-check-cfg=cfg(kor_proxy)");
    let var = |k: &str| std::env::var(k).unwrap_or_default();
    let unsupported = var("CARGO_CFG_TARGET_ENV") == "musl"
        || (var("CARGO_CFG_TARGET_OS") == "windows" && var("CARGO_CFG_TARGET_ARCH") == "aarch64");
    if std::env::var_os("CARGO_FEATURE_PROXY").is_some() && !unsupported {
        println!("cargo::rustc-cfg=kor_proxy");
    }
}
