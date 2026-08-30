//! Minimal async client: send a request, await its response; receive frames and events on
//! channels. Used by the CLI, the integration tests and the A/B test bench — it is the proof
//! that everything is scriptable without the UI.

use anecho_contract::v0::{self as pb, envelope::Payload};
use anecho_wire::{Frame, decode_envelope, encode_envelope, envelope};
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use thiserror::Error;
use tokio::sync::{Mutex, broadcast, mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("websocket: {0}")]
    Ws(#[from] tokio_tungstenite::tungstenite::Error),
    #[error("connection closed")]
    Closed,
    #[error("server error {code:?}: {message}")]
    Server {
        code: pb::ErrorCode,
        message: String,
    },
    #[error("unexpected response: {0}")]
    Unexpected(String),
}

pub type Result<T> = std::result::Result<T, ClientError>;

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Payload>>>>;

#[derive(Debug)]
pub struct Client {
    tx: mpsc::Sender<Message>,
    pending: Pending,
    next_id: AtomicU64,
    frames: broadcast::Sender<Frame>,
    events: broadcast::Sender<pb::Event>,
}

impl Client {
    /// Connect to `ws://host:port/ws`.
    pub async fn connect(url: &str) -> Result<Self> {
        let (ws, _) = tokio_tungstenite::connect_async(url).await?;
        let (mut sink, mut source) = ws.split();
        let (tx, mut rx) = mpsc::channel::<Message>(64);
        let pending: Pending = Arc::default();
        let (frames, _) = broadcast::channel(1024);
        let (events, _) = broadcast::channel(64);

        tokio::spawn(async move {
            while let Some(m) = rx.recv().await {
                if sink.send(m).await.is_err() {
                    break;
                }
            }
            let _ = sink.close().await;
        });

        let p2 = pending.clone();
        let f2 = frames.clone();
        let e2 = events.clone();
        tokio::spawn(async move {
            while let Some(Ok(msg)) = source.next().await {
                let Message::Binary(bytes) = msg else {
                    continue;
                };
                // Frames and envelopes share the binary channel: envelopes always decode as
                // protobuf with a known payload; anything else is tried as a frame.
                if let Ok(env) = decode_envelope(&bytes)
                    && let Some(payload) = env.payload
                {
                    if env.request_id == 0 {
                        if let Payload::Event(ev) = payload {
                            let _ = e2.send(ev);
                        }
                    } else if let Some(tx) = p2.lock().await.remove(&env.request_id) {
                        let _ = tx.send(payload);
                    }
                    continue;
                }
                match Frame::decode(&bytes) {
                    Ok(f) => {
                        let _ = f2.send(f);
                    }
                    Err(e) => log::warn!("undecodable message ({} bytes): {e}", bytes.len()),
                }
            }
            p2.lock().await.clear();
        });

        Ok(Self {
            tx,
            pending,
            next_id: AtomicU64::new(1),
            frames,
            events,
        })
    }

    pub fn frames(&self) -> broadcast::Receiver<Frame> {
        self.frames.subscribe()
    }

    pub fn events(&self) -> broadcast::Receiver<pb::Event> {
        self.events.subscribe()
    }

    /// Send any request payload and await the matching response payload.
    pub async fn request(&self, payload: Payload) -> Result<Payload> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        let env = envelope(id, payload);
        self.tx
            .send(Message::Binary(encode_envelope(&env)))
            .await
            .map_err(|_| ClientError::Closed)?;
        match rx.await.map_err(|_| ClientError::Closed)? {
            Payload::Error(e) => Err(ClientError::Server {
                code: pb::ErrorCode::try_from(e.code).unwrap_or(pb::ErrorCode::Unspecified),
                message: e.message,
            }),
            p => Ok(p),
        }
    }

    pub async fn version(&self) -> Result<pb::GetVersionResponse> {
        match self
            .request(Payload::GetVersion(pb::GetVersionRequest {}))
            .await?
        {
            Payload::Version(v) => Ok(v),
            p => Err(ClientError::Unexpected(format!("{p:?}"))),
        }
    }

    pub async fn list_devices(&self) -> Result<Vec<pb::DeviceInfo>> {
        match self
            .request(Payload::ListDevices(pb::ListDevicesRequest {}))
            .await?
        {
            Payload::Devices(d) => Ok(d.devices),
            p => Err(ClientError::Unexpected(format!("{p:?}"))),
        }
    }

    pub async fn open_session(
        &self,
        device_id: &str,
        config: pb::DeviceConfig,
    ) -> Result<pb::OpenSessionResponse> {
        let req = pb::OpenSessionRequest {
            device_id: device_id.into(),
            config: Some(config),
        };
        match self.request(Payload::OpenSession(req)).await? {
            Payload::SessionOpened(s) => Ok(s),
            p => Err(ClientError::Unexpected(format!("{p:?}"))),
        }
    }

    pub async fn close_session(&self, session_id: u64) -> Result<()> {
        match self
            .request(Payload::CloseSession(pb::CloseSessionRequest {
                session_id,
            }))
            .await?
        {
            Payload::SessionClosed(_) => Ok(()),
            p => Err(ClientError::Unexpected(format!("{p:?}"))),
        }
    }

    pub async fn start_stream(
        &self,
        req: pb::StartStreamRequest,
    ) -> Result<pb::StartStreamResponse> {
        match self.request(Payload::StartStream(req)).await? {
            Payload::StreamStarted(s) => Ok(s),
            p => Err(ClientError::Unexpected(format!("{p:?}"))),
        }
    }

    pub async fn stop_stream(&self, stream_id: u32) -> Result<()> {
        match self
            .request(Payload::StopStream(pb::StopStreamRequest { stream_id }))
            .await?
        {
            Payload::StreamStopped(_) => Ok(()),
            p => Err(ClientError::Unexpected(format!("{p:?}"))),
        }
    }
}
