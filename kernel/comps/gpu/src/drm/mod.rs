use core::fmt::Debug;

use ostd::sync::Mutex;

use crate::drm::mode_config::DrmModeConfig;

pub mod ioctl;
pub mod mode_config;
pub mod mode_object;
pub mod drm_modes;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrmError {
    /// Generic invalid argument or state
    Invalid,
    /// Object not found (CRTC / FB / GEM handle / connector, etc.)
    NotFound,
    /// Operation not supported by this driver / device
    NotSupported,
    /// Resource temporarily unavailable (busy, in use)
    Busy,
    /// Permission or access violation
    PermissionDenied,
    /// Memory allocation or mapping failure
    NoMemory,
}

bitflags::bitflags! {
    pub struct DrmFeatures: u32 {
        const GEM              = 1 << 0;
        const MODESET          = 1 << 1;
        const RENDER           = 1 << 3;
        const ATOMIC           = 1 << 4;
        const SYNCOBJ          = 1 << 5;
        const SYNCOBJ_TIMELINE = 1 << 6;
        const COMPUTE_ACCEL    = 1 << 7;
        const GEM_GPUVA        = 1 << 8;
        const CURSOR_HOTSPOT   = 1 << 9;

        const USE_AGP          = 1 << 25;
        const LEGACY           = 1 << 26;
        const PCI_DMA          = 1 << 27;
        const SG               = 1 << 28;
        const HAVE_DMA         = 1 << 29;
        const HAVE_IRQ         = 1 << 30;
    }
}

pub trait DrmDevice: Debug + Send + Sync {
    fn name(&self) -> &str;
    fn desc(&self) -> &str;
    fn date(&self) -> &str;
    fn features(&self) -> DrmFeatures;
    fn mode_config(&self) -> &Mutex<DrmModeConfig>;
}

impl dyn DrmDevice {
    pub fn check_feature(&self, features: DrmFeatures) -> bool {
        self.features().contains(features)
    }
}
