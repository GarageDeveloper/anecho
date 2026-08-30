//! Manual `Debug` impls for the handle types that wrap non-`Debug` USB
//! endpoints (kept out of the protocol modules to keep those readable).

use crate::device::QA40xDevice;
use crate::transport::{BulkIn, BulkOut};

impl std::fmt::Debug for QA40xDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QA40xDevice")
            .field("virtual", &self.is_virtual())
            .field("keepalive_ok", &self.keepalive_ok_count())
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for BulkOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usb(_) => f.write_str("BulkOut::Usb"),
            #[cfg(feature = "sim")]
            Self::Virt(_) => f.write_str("BulkOut::Virt"),
        }
    }
}

impl std::fmt::Debug for BulkIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usb(_) => f.write_str("BulkIn::Usb"),
            #[cfg(feature = "sim")]
            Self::Virt(_) => f.write_str("BulkIn::Virt"),
        }
    }
}
