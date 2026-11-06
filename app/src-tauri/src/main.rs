// Desktop shell. All logic lives in kb-server; this binary only:
// 1. finds the drive it runs from,
// 2. starts the engine and local web server in-process,
// 3. opens one window on that server,
// 4. stops the model process when the app exits.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;
use tauri::{RunEvent, WebviewUrl, WebviewWindowBuilder};

const PREFERRED_PORT: u16 = 8765;

fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn error_page(msg: &str) -> String {
    let html = format!(
        "<!doctype html><meta charset=utf-8><meta name=color-scheme content='light dark'>\
         <body style='font:17px system-ui;padding:32px;max-width:640px'>\
         <h1>The library could not open</h1><p>{}</p></body>",
        html_escape(msg)
    );
    format!("data:text/html;charset=utf-8,{}", percent_encode(&html))
}

fn start() -> Result<(String, Arc<kb_server::Engine>), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let root = kb_server::find_root(&exe).ok_or_else(|| {
        "This app must run from its library drive (a folder that contains packages and trust). \
         Keep the app on the drive and open it from there."
            .to_string()
    })?;
    let dev = root.dir.join(kb_server::DEV_MARKER).exists() || std::env::var("KB_ALLOW_UNEVALUATED").is_ok();
    // Conversations and memories live on this computer, not on the drive (ADR 0014).
    let store = kb_server::chat::ChatStore::open(&kb_server::chat::default_data_dir()).map_err(|e| e.to_string())?;
    let engine = Arc::new(kb_server::Engine::open(root).map_err(|e| e.to_string())?.with_chat(store));
    let e2 = engine.clone();
    std::thread::spawn(move || {
        e2.start_model(&kb_server::EngineOptions { allow_unevaluated: dev, ..Default::default() });
    });
    // A stable port keeps the window's saved settings between launches.
    let running = kb_server::serve(engine.clone(), &format!("127.0.0.1:{PREFERRED_PORT}"))
        .or_else(|_| kb_server::serve(engine.clone(), "127.0.0.1:0"))
        .map_err(|e| e.to_string())?;
    Ok((running.url, engine))
}

fn main() {
    let started = start();
    let url = match &started {
        Ok((url, _)) => url.clone(),
        Err(msg) => error_page(msg),
    };
    let engine = started.ok().map(|(_, e)| e);

    let app = tauri::Builder::default()
        .setup(move |app| {
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url.parse()?))
                .title("Offline Library")
                .inner_size(1180.0, 820.0)
                .min_inner_size(380.0, 560.0)
                .build()?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build the desktop shell");

    app.run(move |_handle, event| {
        if let RunEvent::Exit = event {
            if let Some(e) = &engine {
                e.shutdown();
            }
        }
    });
}
