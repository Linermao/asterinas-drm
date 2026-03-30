use core::{any::Any, fmt::Debug};

use ostd::sync::Mutex;

use crate::drm::{atomic::DrmAtomicOps, gem::DrmGemOps, kms::DrmKmsOps, objects::DrmObjects};

pub mod atomic;
pub mod drm_modes;
pub mod gem;
pub mod ioctl;
pub mod kms;
pub mod objects;
pub mod syncobj;

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

bitflags::bitflags! {
    pub struct DrmDeviceCaps: u32 {
        const DUMB_CREATE = 1 << 0;
    }
}

pub trait DrmDevice: DrmKmsOps + DrmAtomicOps + DrmGemOps + Debug + Any + Send + Sync {
    fn name(&self) -> &str;
    fn desc(&self) -> &str;
    fn date(&self) -> &str;
    fn features(&self) -> DrmFeatures;
    fn capbilities(&self) -> DrmDeviceCaps;
    
    fn min_width(&self) -> u32;
    fn max_width(&self) -> u32;
    fn min_height(&self) -> u32;
    fn max_height(&self) -> u32;
    fn preferred_depth(&self) -> u32;
    fn prefer_shadow(&self) -> u32;
    fn cursor_width(&self) -> u32;
    fn cursor_height(&self) -> u32;
    fn support_async_page_flip(&self) -> bool;
    fn support_fb_modifiers(&self) -> bool;

    fn objects(&self) -> &Mutex<DrmObjects>;
}

impl dyn DrmDevice {
    pub fn contain_features(&self, features: DrmFeatures) -> bool {
        self.features().contains(features)
    }

    pub fn check_capbility(&self, capbility: DrmDeviceCaps) -> bool {
        self.capbilities().contains(capbility)
    }
}
