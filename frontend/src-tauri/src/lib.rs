//! Kor masaüstü: arayüz `window.kor.*` çağrılarını tek `kor_call` komutuyla bu sürece gönderir.

mod api;
mod api_files;
mod bridge;
// `kor_bytes`: büyük yükler ham bayt olarak (§32.2, ADR 0062).
mod ipc_bytes;
// Performans bindirmesi, perf.log, `kor bench --ui` masaüstü ölçümleri (§32.3, ADR 0062). Dosya ns/ altında
// (IPC şeması sahibi oradan okur) ama kendi `kor_perf` komutu var; ns/mod.rs onu yönlendirmez.
#[path = "ns/perfmon.rs"]
mod perfmon;
// The `kor` CLI the desktop starts: bundled, else the workspace build in debug, else PATH.
mod changes;
mod chat;
mod kor_cli;
// Kor tarayıcı eklentisi: köprü ve yan panel istemcisi (§18, ADR 0058).
mod chrome_bridge;
mod meta;
mod ns;
mod osint;
mod packaging;
// Tepsi, bildirimler, PiP, görev çubuğu, onay kuyruğu (ADR 0051).
mod presence;
mod runtimes;
mod sandbox;
mod state;
mod store;
mod webview_lock;

/// `kor-desktop --terminal <yol>`: "Kor CLI ile aç" terminali açar ve pencere oluşturmadan çıkar (ADR 0052).
pub fn terminal_only() -> bool {
    osint::terminal_only()
}

/// §32.2 (ADR 0062): mimalloc genel ayırıcı.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub fn run() {
    // §32.1 açılış ölçümü bu andan sayılır (perfmon.ready).
    perfmon::mark_start();
    // The bridge dispatcher (`ns::call`, ~70 namespaces) and the agent loop build large futures;
    // debug builds overflowed tokio's default 2 MiB worker stack. Give Tauri a runtime with room.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(16 * 1024 * 1024)
        .build()
        .expect("tokio runtime");
    tauri::async_runtime::set(runtime.handle().clone());
    // Lives for the whole process, like Tauri's own default runtime.
    std::mem::forget(runtime);
    tauri::Builder::default()
        // İlk eklenti olmalı: ikinci açılış argümanlarını (bağlam menüsü, atlama listesi, kor://) bu pencereye iletir.
        .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            osint::second_instance(app, argv, cwd)
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(osint::shortcut_plugin())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        // Yalnızca Rust'tan kullanılır (packaging.rs); arayüze updater izni verilmez.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(state::AppState::default())
        .register_uri_scheme_protocol("kor-sandbox", |_ctx, req| sandbox::respond(&req))
        .setup(|app| {
            // WebView kilidi (§21, ADR 0053): ana pencere devtools/gezinme kilitleriyle burada kurulur.
            webview_lock::create_main_window(app.handle())?;
            // §17.2 IPC izi (ADR 0061): `KOR_IPC_TRACE=<dosya>` ya da `core.ipcTrace`.
            kor_core::ipc_trace::init_from_env();
            bridge::trace_events(app.handle());
            ns::start(app.handle().clone());
            packaging::start(app.handle().clone());
            osint::start(app.handle());
            // İlk açılış entegrasyonları (ADR 0052): dev/test/yalıtılmış çalıştırmalarda hiç çalışmaz.
            osint::first_run(app.handle());
            presence::start(app.handle());
            // §29 (ADR 0054): `web_*` ajan araçları ve Chrome yan panelinde hosting paneli komutları.
            kor_runtime::register_extra_tools(kor_web::site::tools::all);
            kor_tools::chrome::set_panel_handler(kor_web::site::panels::handler);
            chrome_bridge::start(app.handle().clone());
            Ok(())
        })
        .on_window_event(presence::window_event)
        .on_window_event(webview_lock::on_window_event)
        .invoke_handler(tauri::generate_handler![
            bridge::kor_call,
            ipc_bytes::kor_bytes,
            perfmon::kor_perf
        ])
        .build(tauri::generate_context!())
        .expect("Kor masaüstü başlatılamadı")
        .run(|_, event| {
            if let tauri::RunEvent::Exit = event {
                presence::shutdown();
                ns::shutdown();
            }
        });
}
