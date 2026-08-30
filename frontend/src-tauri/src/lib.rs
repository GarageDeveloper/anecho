//! Desktop shell. The window is a plain web client of the Anecho API; the only thing this
//! crate adds is starting the headless backend in-process so the app runs standalone.
//! Nothing here computes or caches measurement data (CLAUDE.md rule 2).

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Where the embedded backend listens. The webview connects to `ws://<addr>/ws`.
pub const BACKEND_ADDR: ([u8; 4], u16) = ([127, 0, 0, 1], anecho_server::DEFAULT_PORT);

/// Idempotent safe teardown: close every session so the devices are released (a QA40x is
/// left on its safe input range with the stream stopped). Every exit path funnels here —
/// menu Quit, window close, Ctrl-C in the terminal — like qa40x-rs's safe_shutdown.
fn safe_shutdown(engine: &Arc<anecho_engine::Engine>) {
    static DONE: AtomicBool = AtomicBool::new(false);
    if DONE.swap(true, Ordering::SeqCst) {
        return;
    }
    log::info!("exit: safe teardown");
    let engine = engine.clone();
    let result = tauri::async_runtime::block_on(async move {
        tokio::time::timeout(std::time::Duration::from_secs(10), engine.shutdown()).await
    });
    match result {
        Ok(()) => log::info!("exit: devices released"),
        Err(_) => log::warn!("exit: teardown timed out after 10 s"),
    }
}

pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let engine = anecho::build_engine(&anecho::BackendOptions {
        virtual_loopback: false,
        cpal: true,
        qa40x: true,
        qa40x_sim: false,
    });

    let app = tauri::Builder::default()
        .setup({
            let engine = engine.clone();
            move |app| {
                let addr = SocketAddr::from(BACKEND_ADDR);
                let server_engine = engine.clone();
                tauri::async_runtime::spawn(async move {
                    match anecho_server::serve(server_engine, addr, std::future::pending()).await {
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
                // Terminal Ctrl-C (tauri dev) exits through the same path as menu Quit.
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    if tokio::signal::ctrl_c().await.is_ok() {
                        log::info!("exit: Ctrl-C");
                        handle.exit(0);
                    }
                });
                Ok(())
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building the Anecho desktop app");

    app.run(move |_handle, event| {
        if matches!(
            event,
            tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
        ) {
            safe_shutdown(&engine);
        }
    });
}
