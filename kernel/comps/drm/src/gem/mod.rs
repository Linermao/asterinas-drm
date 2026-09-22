// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::fmt::Debug;

use aster_core::prelude::*;

use crate::{device::DrmDevice, gem::object::DrmGemObject};

pub mod object;
pub mod shmem;
pub mod vma_manager;

pub trait DrmGemOps: Debug + DrmDevice + Send + Sync {
    fn create_dumb(&self, _width: u32, _height: u32, _bpp: u32) -> Result<Arc<dyn DrmGemObject>> {
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "dumb-buffer creation is not supported by this DRM driver"
        )
    }
}
