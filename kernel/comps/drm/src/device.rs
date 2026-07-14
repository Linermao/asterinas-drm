// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, sync::Arc};
use core::{any::Any, fmt::Debug};

use ostd_pod::Pod;
use zerocopy::{FromZeros, IntoBytes};

use crate::{
    DrmError, DrmFence, DrmSyncObj,
    atomic::DrmAtomicOps,
    gem::{DrmGemOps, DrmIoctlGemCtx, vma_manager::DrmVmaOffsetManager},
    kms::DrmKmsOps,
};

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

pub trait DrmIoctlCommandCtx: DrmIoctlGemCtx + Debug {
    fn device_private(&self) -> Option<&dyn DrmDevicePrivate>;
    fn export_fence(&self, fence: Arc<DrmFence>) -> Result<i32, DrmError>;
    fn import_fence(&self, fd: i32) -> Result<Arc<DrmFence>, DrmError>;
    fn lookup_syncobj(&self, handle: u32) -> Result<Arc<DrmSyncObj>, DrmError>;
    fn read_user_bytes(&self, addr: usize, buf: &mut [u8]) -> Result<(), DrmError>;
    fn write_user_bytes(&self, addr: usize, buf: &[u8]) -> Result<(), DrmError>;
}

impl dyn DrmIoctlCommandCtx + '_ {
    pub fn read_ioctl_arg<T: FromZeros + IntoBytes + Pod>(
        &self,
        arg: usize,
    ) -> Result<T, DrmError> {
        let mut value = T::new_zeroed();
        self.read_user_bytes(arg, value.as_mut_bytes())?;
        Ok(value)
    }
}

pub trait DrmDevicePrivate: Any + Debug + Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn release(&self);
}

/// Defines the top-level contract of a DRM device instance.
///
/// `DrmDevice` is the composition root for device-facing DRM behavior.
/// It provides stable identity metadata and shared capability discovery,
/// while higher-level DRM operations are expected to be layered as
/// dedicated operation traits.
///
pub trait DrmDevice: DrmKmsOps + DrmGemOps + DrmAtomicOps + Debug + Send + Sync {
    fn name(&self) -> &str;
    fn desc(&self) -> &str;
    fn bus_info(&self) -> Option<DrmDeviceBusInfo> {
        None
    }
    fn features(&self) -> &DrmFeatures;
    fn caps(&self) -> &DrmDeviceCaps;
    fn vma_manager(&self) -> &DrmVmaOffsetManager;
    fn create_private(&self) -> Result<Option<Box<dyn DrmDevicePrivate>>, DrmError> {
        Ok(None)
    }
    fn handle_command(
        &self,
        _cmd: u32,
        _arg: usize,
        _ctx: &dyn DrmIoctlCommandCtx,
    ) -> Result<(), DrmError> {
        Err(DrmError::NotFound)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DrmDeviceBusInfo {
    Pci { vendor_id: u16, device_id: u16 },
}

impl dyn DrmDevice {
    pub fn has_feature(&self, feature: DrmFeatures) -> bool {
        self.features().contains(feature)
    }
}

#[derive(Debug)]
pub struct DrmDeviceCaps {
    min_fb_width_px: u32,
    max_fb_width_px: u32,
    min_fb_height_px: u32,
    max_fb_height_px: u32,
    preferred_color_depth_bpp: u32,
    cursor_width_px: u32,
    cursor_height_px: u32,

    /// Indicates whether dumb-buffer should prefer shadow-buffer rendering.
    prefer_shadow_buffer: bool,
    has_async_page_flip: bool,
    /// This field mainly exists for legacy compatibility and is the positive form of
    /// Linux `fb_modifiers_not_supported`.
    has_fb_modifiers: bool,

    /// Blows are an Asterinas-specific capability check used by this project and
    /// is not treated as a direct Linux capability query in this abstraction.
    has_dumb_buffer: bool,
    has_flip_target: bool,
}

impl DrmDeviceCaps {
    /// Creates device capability values with validated geometry ranges.
    ///
    /// Returns an error when `min_width >= max_width` or `min_height >= max_height`.
    pub fn new(
        min_fb_width_px: u32,
        max_fb_width_px: u32,
        min_fb_height_px: u32,
        max_fb_height_px: u32,
        preferred_color_depth_bpp: u32,
        cursor_width_px: u32,
        cursor_height_px: u32,
        prefer_shadow_buffer: bool,
        has_async_page_flip: bool,
        has_fb_modifiers: bool,
        has_dumb_buffer: bool,
        has_flip_target: bool,
    ) -> Result<Self, DrmError> {
        if min_fb_width_px >= max_fb_width_px {
            return Err(DrmError::Invalid);
        }

        if min_fb_height_px >= max_fb_height_px {
            return Err(DrmError::Invalid);
        }

        Ok(Self {
            min_fb_width_px,
            max_fb_width_px,
            min_fb_height_px,
            max_fb_height_px,
            preferred_color_depth_bpp,
            cursor_width_px,
            cursor_height_px,
            prefer_shadow_buffer,
            has_async_page_flip,
            has_fb_modifiers,
            has_dumb_buffer,
            has_flip_target,
        })
    }

    pub fn min_fb_width_px(&self) -> u32 {
        self.min_fb_width_px
    }

    pub fn max_fb_width_px(&self) -> u32 {
        self.max_fb_width_px
    }

    pub fn min_fb_height_px(&self) -> u32 {
        self.min_fb_height_px
    }

    pub fn max_fb_height_px(&self) -> u32 {
        self.max_fb_height_px
    }

    pub fn preferred_color_depth_px(&self) -> u32 {
        self.preferred_color_depth_bpp
    }

    pub fn cursor_width_px(&self) -> u32 {
        self.cursor_width_px
    }

    pub fn cursor_height_px(&self) -> u32 {
        self.cursor_height_px
    }

    pub fn prefer_shadow_buffer(&self) -> bool {
        self.prefer_shadow_buffer
    }

    pub fn has_async_page_flip(&self) -> bool {
        self.has_async_page_flip
    }

    pub fn has_fb_modifiers(&self) -> bool {
        self.has_fb_modifiers
    }

    pub fn has_dumb_buffer(&self) -> bool {
        self.has_dumb_buffer
    }

    pub fn has_flip_target(&self) -> bool {
        self.has_flip_target
    }
}

impl Default for DrmDeviceCaps {
    fn default() -> Self {
        Self {
            min_fb_width_px: 1,
            max_fb_width_px: 4096,
            min_fb_height_px: 1,
            max_fb_height_px: 4096,
            preferred_color_depth_bpp: 24,
            cursor_width_px: 64,
            cursor_height_px: 64,
            has_async_page_flip: false,
            // Match the current DRM/KMS implementation: framebuffer modifiers
            // are not supported yet, so userspace must stay on the non-modifier
            // `ADDFB2` path.
            has_fb_modifiers: false,
            prefer_shadow_buffer: true,
            has_dumb_buffer: true,
            has_flip_target: false,
        }
    }
}
