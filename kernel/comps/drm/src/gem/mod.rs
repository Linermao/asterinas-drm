// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::fmt::Debug;

use crate::{DrmError, gem::object::DrmGemObject};

pub mod object;
pub mod vma_manager;

pub trait DrmIoctlGemCtx: Send + Sync {
    fn create_shmem_gem(&self, size: usize, pitch: u32) -> Result<Arc<dyn DrmGemObject>, DrmError>;
    fn add_gem_object(&self, gem_object: Arc<dyn DrmGemObject>) -> Result<u32, DrmError>;
    fn replace_gem_object(
        &self,
        handle: u32,
        gem_object: Arc<dyn DrmGemObject>,
    ) -> Result<(), DrmError>;
    fn lookup_gem_object(&self, handle: u32) -> Option<Arc<dyn DrmGemObject>>;
    fn map_gem_handle(&self, handle: u32) -> Result<u64, DrmError>;
}

pub trait DrmGemOps: Debug + Send + Sync {
    fn create_dumb(
        &self,
        _width: u32,
        _height: u32,
        _bpp: u32,
        _ctx: &dyn DrmIoctlGemCtx,
    ) -> Result<Arc<dyn DrmGemObject>, DrmError> {
        Err(DrmError::NotSupported)
    }
}

#[derive(Debug)]
pub struct DrmSgEntry {
    addr: u64,
    length: u32,
}

impl DrmSgEntry {
    pub fn new(addr: u64, length: u32) -> Self {
        Self { addr, length }
    }

    pub fn addr(&self) -> u64 {
        self.addr
    }

    pub fn length(&self) -> u32 {
        self.length
    }

    pub fn update_length(&mut self, length: u32) {
        self.length = length
    }
}
