use alloc::{boxed::Box, sync::Arc, vec::Vec};
use core::{
    any::Any,
    fmt::Debug,
    sync::atomic::{AtomicU64, Ordering},
};

use hashbrown::HashMap;
use ostd::{mm::PAGE_SIZE, sync::Mutex};

use crate::drm::{
    atomic::{
        DrmAtomicHelper, DrmAtomicPendingState,
        vblank::{PageFlipEvent, VblankCallback},
    },
    gem::{DrmGemBackend, DrmGemObject},
    ioctl::{DrmModeCreateDumb, DrmModeCrtc, DrmModeCrtcPageFlip, DrmModeFbCmd2},
    mode_config::{DrmModeConfig, ObjectId},
};

pub mod atomic;
pub mod drm_modes;
pub mod gem;
pub mod ioctl;
pub mod mode_config;
pub mod mode_object;

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

pub type MemfdAllocatorType = fn(&str, u64) -> Result<Box<dyn DrmGemBackend>, DrmError>;

#[derive(Debug)]
pub struct VmaOffsetManager {
    base: u64,
    next_offset: AtomicU64,
    offset_to_handle: HashMap<u64, u32>,
}

impl VmaOffsetManager {
    pub fn new() -> Self {
        Self {
            base: 0x10_0000,
            next_offset: AtomicU64::new(0),
            offset_to_handle: HashMap::new(),
        }
    }

    pub fn alloc(&mut self, handle: u32) -> Result<u64, DrmError> {
        let offset = self.base
            + self
                .next_offset
                .fetch_add(PAGE_SIZE as u64, Ordering::SeqCst);
        self.offset_to_handle.insert(offset, handle);
        Ok(offset)
    }

    pub fn lookup(&self, offset: u64) -> Option<u32> {
        self.offset_to_handle.get(&offset).copied()
    }

    pub fn free(&mut self, offset: u64) {
        self.offset_to_handle.remove(&offset);
    }
}

pub trait DrmDevice: Debug + Any + Send + Sync {
    fn name(&self) -> &str;
    fn desc(&self) -> &str;
    fn date(&self) -> &str;
    fn features(&self) -> DrmFeatures;
    fn capbilities(&self) -> DrmDeviceCaps;
    fn mode_config(&self) -> &Mutex<DrmModeConfig>;
    fn vma_offset_manager(&self) -> &Mutex<VmaOffsetManager>;
    fn set_config(
        &self,
        crtc_resp: &DrmModeCrtc,
        connector_ids: Vec<ObjectId>,
    ) -> Result<(), DrmError>;
    fn page_flip(
        &self,
        page_flip: &DrmModeCrtcPageFlip,
        vblank_callback: Arc<dyn VblankCallback>,
        target: Option<u32>,
    ) -> Result<(), DrmError>;
    fn dirty_framebuffer(&self, fb_id: ObjectId) -> Result<(), DrmError>;
    fn create_dumb(
        &self,
        _args: &DrmModeCreateDumb,
        _memfd_allocator: MemfdAllocatorType,
    ) -> Result<Arc<dyn DrmGemObject>, DrmError> {
        Err(DrmError::NotSupported)
    }
    fn map_dumb(&self, _handle: u32) -> Result<u64, DrmError> {
        Err(DrmError::NotSupported)
    }
    fn fb_create(
        &self,
        fb_cmd: &DrmModeFbCmd2,
        gem_object: Arc<dyn DrmGemObject>,
    ) -> Result<ObjectId, DrmError>;
    fn atomic_commit(
        &self,
        nonblock: bool,
        pending_state: &mut DrmAtomicPendingState,
        page_flip_event: Option<PageFlipEvent>,
    ) -> Result<(), DrmError>;
    fn atomic_commit_tail(&self, pending_state: &mut DrmAtomicPendingState)
    -> Result<(), DrmError>;
}

impl dyn DrmDevice {
    pub fn check_feature(&self, features: DrmFeatures) -> bool {
        self.features().contains(features)
    }

    pub fn check_capbility(&self, capbility: DrmDeviceCaps) -> bool {
        self.capbilities().contains(capbility)
    }

    pub fn lookup_gem_handle(&self, offset: usize) -> Option<u32> {
        self.vma_offset_manager().lock().lookup(offset as u64)
    }
}
