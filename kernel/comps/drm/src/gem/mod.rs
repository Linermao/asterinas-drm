// SPDX-License-Identifier: MPL-2.0

//! Graphics Execution Manager (GEM) support.
//!
//! GEM provides DRM drivers with graphics buffer objects
//! and the core mechanisms needed to expose those objects to userspace.
//! A [`DrmGemObject`] owns a driver-provided memory backend,
//! while the DRM core manages per-file handles, mmap access,
//! and device-wide fake mmap offsets.
//!
//! A driver enables GEM by returning its [`DrmGemOps`] implementation from
//! [`crate::device::DrmDevice::as_gem_ops`].
//! Drivers may use [`shmem::DrmGemShmemBackend`] for RAM-backed objects
//! or provide their own [`object::DrmGemObjectBackend`] implementation.

use alloc::sync::Arc;
use core::fmt::Debug;

use aster_core::prelude::*;

use crate::gem::object::DrmGemObject;

pub(crate) mod mmap_offset;
pub mod object;
pub(crate) mod prime;
pub mod shmem;

/// Operations provided by a GEM-capable DRM driver.
///
/// A device exposes these operations through
/// [`crate::device::DrmDevice::as_gem_ops`].
/// Every GEM-capable driver implements [`DrmGemOps::create_dumb`]. A driver
/// without dumb-buffer support returns `EOPNOTSUPP`.
pub trait DrmGemOps: Debug + Send + Sync {
    /// Creates a dumb GEM buffer with the requested size.
    fn create_dumb(&self, size: usize) -> Result<Arc<DrmGemObject>>;

    /// Returns whether GEM objects support the minimal PRIME sharing interface.
    ///
    /// Objects must be CPU-coherent, support shared mappings, and have no
    /// outstanding asynchronous device accesses. Imports are restricted to
    /// objects exported from the same registered DRM device.
    fn supports_prime(&self) -> bool {
        false
    }
}
