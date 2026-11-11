fn main() {
    // `kor_call` uygulama komutu olarak bildirilir: yalnızca `allow-kor-call` yetkisi olan webview (ana
    // arayüz) çağırabilir. Önizleme/ajan tarayıcısı alt webview'leri ve onlara gömülen sayfalar çağıramaz.
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(&["kor_call", "kor_bytes", "kor_perf"])),
    )
    .expect("Tauri derleme betiği başarısız");
}
