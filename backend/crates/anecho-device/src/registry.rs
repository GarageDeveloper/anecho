use crate::{
    BackendKind, DeviceBackend, DeviceDescriptor, DeviceError, DeviceId, MeasurementDevice, Result,
};
use std::sync::Arc;

/// Aggregates every configured backend behind a single enumerate/open surface.
#[derive(Default)]
pub struct DeviceRegistry {
    backends: Vec<Arc<dyn DeviceBackend>>,
}

impl std::fmt::Debug for DeviceRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kinds: Vec<BackendKind> = self.backends.iter().map(|b| b.kind()).collect();
        f.debug_struct("DeviceRegistry")
            .field("backends", &kinds)
            .finish()
    }
}

impl DeviceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_backend(mut self, backend: Arc<dyn DeviceBackend>) -> Self {
        self.backends.push(backend);
        self
    }

    pub fn backends(&self) -> &[Arc<dyn DeviceBackend>] {
        &self.backends
    }

    /// Every device of every backend, in backend registration order.
    pub async fn enumerate(&self) -> Vec<DeviceDescriptor> {
        let mut out = Vec::new();
        for b in &self.backends {
            out.extend(b.enumerate().await);
        }
        out
    }

    pub async fn open(&self, id: &DeviceId) -> Result<Box<dyn MeasurementDevice>> {
        let kind = id
            .backend()
            .ok_or_else(|| DeviceError::NotFound(id.to_string()))?;
        let backend = self
            .backends
            .iter()
            .find(|b| b.kind() == kind)
            .ok_or_else(|| DeviceError::NotFound(id.to_string()))?;
        backend.open(id).await
    }
}
