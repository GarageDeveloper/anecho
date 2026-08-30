//! Desktop shell. The window is a plain web client of the Anecho API; the only thing this
//! crate adds is starting the headless backend in-process so the app runs standalone.
//! Nothing here computes or caches measurement data (CLAUDE.md rule 2).

use std::net::SocketAddr;

/// Where the embedded backend listens. The webview connects to `ws://<addr>/ws`.
pub const BACKEND_ADDR: ([u8; 4], u16) = ([127, 0, 0, 1], anecho_server::DEFAULT_PORT);

pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    tauri::Builder::default()
        .setup(|_app| {
            let engine = anecho::build_engine(&anecho::BackendOptions {
                virtual_loopback: false,
                cpal: true,
                qa40x: true,
                qa40x_sim: false,
            });
            let addr = SocketAddr::from(BACKEND_ADDR);
            tauri::async_runtime::spawn(async move {
                match anecho_server::serve(engine, addr, std::future::pending()).await {
                    Ok((bound, task)) => {
                        log::info!("anecho backend listening on ws://{bound}/ws");
                        if let Err(e) = task.await {
                            log::error!("backend stopped: {e}");
                        }
                    }
                    Err(e) => log::error!(
                        "cannot start the backend on {addr}: {e} (is another anecho running?)"
                    ),
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the Anecho desktop app");
}
