//! WebSocket front for the engine. One connection = one client; requests are answered in
//! order, frames of streams started by this connection are pushed as binary messages,
//! events are broadcast to every connection.

pub mod convert;

use anecho_contract::v0::{self as pb, envelope::Payload};
use anecho_engine::{Engine, EngineError, Event};
use anecho_wire::{decode_envelope, encode_envelope, envelope};
use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use futures_util::{SinkExt, StreamExt};
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::broadcast;

/// Default TCP port of the Anecho API.
pub const DEFAULT_PORT: u16 = 4800;

pub fn router(engine: Arc<Engine>) -> Router {
    Router::new()
        .route("/ws", get(ws_handler))
        .route(
            "/",
            get(|| async { "anecho backend — WebSocket API at /ws" }),
        )
        .with_state(engine)
}

/// Bind and serve until `shutdown` completes. Returns the bound address once listening.
pub async fn serve(
    engine: Arc<Engine>,
    addr: SocketAddr,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> std::io::Result<(SocketAddr, tokio::task::JoinHandle<std::io::Result<()>>)> {
    let listener = TcpListener::bind(addr).await?;
    let bound = listener.local_addr()?;
    let app = router(engine.clone());
    let task = tokio::spawn(async move {
        let r = axum::serve(listener, app)
            .with_graceful_shutdown(shutdown)
            .await;
        engine.shutdown().await;
        r
    });
    Ok((bound, task))
}

async fn ws_handler(ws: WebSocketUpgrade, State(engine): State<Arc<Engine>>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| connection(socket, engine))
}

async fn connection(socket: WebSocket, engine: Arc<Engine>) {
    let (mut sink, mut source) = socket.split();
    let mut frames = engine.frames();
    let mut events = engine.events();
    // Streams and sessions owned by this connection, cleaned up on close.
    let mut my_streams: HashSet<u32> = HashSet::new();
    let mut my_sessions: HashSet<u64> = HashSet::new();

    loop {
        tokio::select! {
            msg = source.next() => {
                let Some(Ok(msg)) = msg else { break };
                let bytes = match msg {
                    Message::Binary(b) => b,
                    Message::Close(_) => break,
                    _ => continue,
                };
                let env = match decode_envelope(&bytes) {
                    Ok(e) => e,
                    Err(e) => {
                        let err = error_env(0, pb::ErrorCode::BadRequest, &format!("undecodable envelope: {e}"));
                        if sink.send(Message::Binary(encode_envelope(&err))).await.is_err() { break; }
                        continue;
                    }
                };
                let reply = handle(&engine, env, &mut my_streams, &mut my_sessions).await;
                if sink.send(Message::Binary(encode_envelope(&reply))).await.is_err() { break; }
            }
            frame = frames.recv() => {
                match frame {
                    Ok(f) if my_streams.contains(&f.stream_id) => {
                        if sink.send(Message::Binary(f.encode())).await.is_err() { break; }
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(n)) => log::warn!("connection lagged, {n} frames skipped"),
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            ev = events.recv() => {
                match ev {
                    Ok(Event::StreamEnded { stream_id }) => { my_streams.remove(&stream_id); }
                    Ok(ev) => {
                        let mine = match &ev {
                            Event::StreamOverrun { stream_id, .. } => my_streams.contains(stream_id),
                            Event::RangeChanged { session_id, .. } => my_sessions.contains(session_id),
                            Event::StreamEnded { .. } => false,
                        };
                        if mine && let Some(pb_ev) = convert::event(&ev) {
                            let env = envelope(0, Payload::Event(pb_ev));
                            if sink.send(Message::Binary(encode_envelope(&env))).await.is_err() { break; }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
    for id in my_sessions {
        let _ = engine.close_session(id).await;
    }
}

async fn handle(
    engine: &Engine,
    env: pb::Envelope,
    my_streams: &mut HashSet<u32>,
    my_sessions: &mut HashSet<u64>,
) -> pb::Envelope {
    let id = env.request_id;
    let Some(payload) = env.payload else {
        return error_env(id, pb::ErrorCode::BadRequest, "empty envelope");
    };
    let result: Result<Payload, EngineError> = match payload {
        Payload::GetVersion(_) => Ok(Payload::Version(pb::GetVersionResponse {
            backend_version: anecho_engine::VERSION.into(),
            contract_version: anecho_engine::CONTRACT_VERSION.into(),
        })),
        Payload::ListDevices(_) => Ok(Payload::Devices(pb::ListDevicesResponse {
            devices: engine
                .list_devices()
                .await
                .iter()
                .map(convert::device_info)
                .collect(),
        })),
        Payload::OpenSession(req) => {
            let cfg = req
                .config
                .ok_or_else(|| EngineError::BadRequest("config is required".into()))
                .and_then(|c| convert::device_config(&c));
            match cfg {
                Ok(cfg) => engine
                    .open_session(&anecho_device::DeviceId(req.device_id), cfg)
                    .await
                    .map(|(session_id, applied)| {
                        my_sessions.insert(session_id);
                        Payload::SessionOpened(pb::OpenSessionResponse {
                            session_id,
                            applied: Some(convert::applied_config(&applied)),
                        })
                    }),
                Err(e) => Err(e),
            }
        }
        Payload::CloseSession(req) => engine.close_session(req.session_id).await.map(|()| {
            my_sessions.remove(&req.session_id);
            Payload::SessionClosed(pb::CloseSessionResponse {})
        }),
        Payload::StartStream(req) => match convert::stream_request(&req) {
            Ok(sr) => engine.start_stream(req.session_id, sr).await.map(|info| {
                my_streams.insert(info.stream_id);
                Payload::StreamStarted(convert::stream_started(&info))
            }),
            Err(e) => Err(e),
        },
        Payload::StopStream(req) => engine.stop_stream(req.stream_id).await.map(|()| {
            my_streams.remove(&req.stream_id);
            Payload::StreamStopped(pb::StopStreamResponse {})
        }),
        other => Err(EngineError::BadRequest(format!("not a request: {other:?}"))),
    };
    match result {
        Ok(p) => envelope(id, p),
        Err(e) => error_env(id, convert::error_code(&e), &e.to_string()),
    }
}

fn error_env(id: u64, code: pb::ErrorCode, message: &str) -> pb::Envelope {
    envelope(
        id,
        Payload::Error(pb::Error {
            code: code as i32,
            message: message.into(),
        }),
    )
}
