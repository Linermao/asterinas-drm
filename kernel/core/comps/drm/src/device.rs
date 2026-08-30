// SPDX-License-Identifier: MPL-2.0

use alloc::{
    collections::BTreeMap,
    sync::{Arc, Weak},
};
use core::{
    fmt::Debug,
    sync::atomic::{AtomicBool, Ordering},
};

use ostd::sync::Mutex;
use sparse_id_alloc::SparseIdAlloc;

use crate::{DrmError, DrmRect};

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

#[derive(Debug)]
pub struct DrmDeviceState {
    master: Mutex<Option<DrmMaster>>,
}

impl Default for DrmDeviceState {
    fn default() -> Self {
        Self {
            master: Mutex::new(None),
        }
    }
}

/// State shared by primary files that were opened under the same DRM master.
#[derive(Debug)]
pub struct DrmMaster {
    client_id: u64,
    allocator: SparseIdAlloc,
    magic_table: BTreeMap<u32, Weak<AtomicBool>>,
}

impl DrmMaster {
    fn new(client_id: u64) -> Self {
        Self {
            client_id,
            allocator: SparseIdAlloc::new(1, u32::MAX),
            magic_table: BTreeMap::new(),
        }
    }

    pub fn allocate_magic(&mut self, target: &Arc<AtomicBool>) -> Result<u32, DrmError> {
        let magic = self.allocator.alloc().ok_or(DrmError::NoMemory)?;
        self.magic_table.insert(magic, Arc::downgrade(target));
        Ok(magic)
    }

    pub fn authenticate_magic(&mut self, magic: u32) -> Result<(), DrmError> {
        let target = self
            .magic_table
            .remove(&magic)
            .and_then(|target| target.upgrade())
            .ok_or(DrmError::Invalid)?;

        target.store(true, Ordering::Release);
        Ok(())
    }

    pub fn release_magic(&mut self, magic: u32) {
        self.magic_table.remove(&magic);
        self.allocator.free(magic);
    }
}

/// Defines the top-level contract of a DRM device instance.
///
/// `DrmDevice` is the composition root for device-facing DRM behavior.
/// It provides stable identity metadata and shared capability discovery,
/// while higher-level DRM operations are expected to be layered as
/// dedicated operation traits.
///
pub trait DrmDevice: Debug + Send + Sync {
    fn name(&self) -> &str;
    fn desc(&self) -> &str;
    fn features(&self) -> &DrmFeatures;
    fn caps(&self) -> &DrmDeviceCaps;
    fn state(&self) -> &DrmDeviceState;
}

impl dyn DrmDevice {
    pub fn has_feature(&self, feature: DrmFeatures) -> bool {
        self.features().contains(feature)
    }

    pub fn is_current_master(&self, client_id: u64) -> bool {
        let master = self.state().master.lock();
        master
            .as_ref()
            .is_some_and(|master| master.client_id == client_id)
    }

    /// Associates a primary client with the current master context.
    ///
    /// If no master exists, the client creates a new context and becomes master.
    pub fn open_primary_client(&self, client_id: u64) -> bool {
        let mut master = self.state().master.lock();
        match master.as_ref() {
            Some(_) => false,
            None => {
                let new_master = DrmMaster::new(client_id);
                *master = Some(new_master);
                true
            }
        }
    }

    /// Makes a primary client the current DRM master.
    ///
    /// A client becoming master for the first time starts a new master context.
    pub fn set_master(&self, client_id: u64) -> Result<(), DrmError> {
        let mut master = self.state().master.lock();
        match master.as_ref() {
            Some(current_master) => {
                if current_master.client_id == client_id {
                    Ok(())
                } else {
                    Err(DrmError::Busy)
                }
            }
            None => {
                let context = DrmMaster::new(client_id);
                *master = Some(context);
                Ok(())
            }
        }
    }

    pub fn drop_master(&self, client_id: u64) -> Result<(), DrmError> {
        let mut master = self.state().master.lock();
        if !master
            .as_ref()
            .is_some_and(|master| master.client_id == client_id)
        {
            return Err(DrmError::Invalid);
        }
        *master = None;

        Ok(())
    }

    pub fn allocate_magic(&self, target: &Arc<AtomicBool>) -> Result<u32, DrmError> {
        let mut master = self.state().master.lock();
        if let Some(master) = master.as_mut() {
            return master.allocate_magic(target);
        } else {
            return Err(DrmError::Invalid);
        }
    }

    pub fn authenticate_magic(&self, magic: u32) -> Result<(), DrmError> {
        let mut master = self.state().master.lock();
        if let Some(master) = master.as_mut() {
            return master.authenticate_magic(magic);
        } else {
            return Err(DrmError::Invalid);
        }
    }

    pub fn release_magic(&self, magic: u32) -> Result<(), DrmError> {
        let mut master = self.state().master.lock();
        if let Some(master) = master.as_mut() {
            master.release_magic(magic);
        } else {
            return Err(DrmError::Invalid);
        }

        Ok(())
    }
}

bitflags::bitflags! {
    pub struct DrmDeviceCapFlags: u32 {
        const ASYNC_PAGE_FLIP       = 1 << 0;
        /// This field mainly exists for legacy compatibility and is the positive form of
        /// Linux `fb_modifiers_not_supported`.
        const FB_MODIFIERS          = 1 << 1;
        /// Indicates whether dumb-buffer should prefer shadow-buffer rendering.
        const SHADOW_BUFFER         = 1 << 2;
        // Blows are an Asterinas-specific capability check used by this project and
        // is not treated as a direct Linux capability query in this abstraction.
        const DUMB_BUFFER           = 1 << 3;
        const PAGE_FLIP_TARGET      = 1 << 4;
    }
}

#[derive(Debug)]
pub struct DrmDeviceCaps {
    preferred_color_depth: u32,
    min_fb_rect: DrmRect,
    max_fb_rect: DrmRect,
    cursor_rect: DrmRect,

    flags: DrmDeviceCapFlags,
}

impl DrmDeviceCaps {
    /// Creates device capability values with validated geometry ranges.
    pub fn new(
        preferred_color_depth: u32,
        min_fb_rect: DrmRect,
        max_fb_rect: DrmRect,
        cursor_rect: DrmRect,
        flags: DrmDeviceCapFlags,
    ) -> Result<Self, DrmError> {
        if !max_fb_rect.contains_rect(&min_fb_rect) {
            return Err(DrmError::Invalid);
        }

        Ok(Self {
            preferred_color_depth,
            min_fb_rect,
            max_fb_rect,
            cursor_rect,
            flags,
        })
    }

    pub fn min_fb_rect(&self) -> DrmRect {
        self.min_fb_rect
    }

    pub fn max_fb_rect(&self) -> DrmRect {
        self.max_fb_rect
    }

    pub fn cursor_rect(&self) -> DrmRect {
        self.cursor_rect
    }

    pub fn preferred_color_depth(&self) -> u32 {
        self.preferred_color_depth
    }

    pub fn flags(&self) -> DrmDeviceCapFlags {
        self.flags
    }
}

impl Default for DrmDeviceCaps {
    fn default() -> Self {
        Self {
            preferred_color_depth: 24,
            min_fb_rect: DrmRect::new(0, 0, 1, 1),
            max_fb_rect: DrmRect::new(0, 0, 4096, 4096),
            cursor_rect: DrmRect::new(0, 0, 64, 64),
            // TODO: Add FLIP_TARGET after finish page_flip with target.
            flags: DrmDeviceCapFlags::DUMB_BUFFER,
        }
    }
}
