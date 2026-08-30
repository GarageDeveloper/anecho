//! Manual `Debug` impls for the simulator-backed types (`sim` feature).

impl std::fmt::Debug for crate::transport::VirtEp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VirtEp")
            .field("pending", &self.pending())
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for crate::discovery::virt::VirtualUnit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VirtualUnit").finish_non_exhaustive()
    }
}

impl std::fmt::Debug for crate::discovery::virt::VirtualDeviceSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VirtualDeviceSource")
            .field("units", &self.units().len())
            .finish_non_exhaustive()
    }
}
