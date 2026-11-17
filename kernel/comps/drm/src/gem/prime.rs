// SPDX-License-Identifier: MPL-2.0

//! PRIME sharing of the current device's CPU-coherent GEM objects.
//!
//! An exported dma-buf retains the GEM object and its registered DRM device.
//! Importing on the same device recovers that object, allowing another DRM file
//! to create its own handle without duplicating the backing pages. Cross-device
//! DMA attachment and asynchronous device accesses are not supported.

use alloc::{boxed::Box, sync::Arc};

use aster_core::{
    fs::{
        dma_buf::DmaBuf,
        file::{Mappable, MappedObject},
    },
    prelude::*,
    vm::vmar::MapHandle,
};

use crate::{device::RegisteredDrmDevice, gem::object::DrmGemObject};

#[derive(Debug)]
pub(crate) struct DrmPrimeBuffer {
    object: Arc<DrmGemObject>,
    device: Arc<RegisteredDrmDevice>,
}

impl DrmPrimeBuffer {
    pub(crate) fn new(device: Arc<RegisteredDrmDevice>, object: Arc<DrmGemObject>) -> Self {
        Self { object, device }
    }

    pub(crate) fn object(&self, device: &Arc<RegisteredDrmDevice>) -> Result<Arc<DrmGemObject>> {
        if !Arc::ptr_eq(&self.device, device) {
            return_errno_with_message!(
                Errno::EOPNOTSUPP,
                "cross-device PRIME imports are not supported"
            );
        }
        Ok(self.object.clone())
    }
}

impl DmaBuf for DrmPrimeBuffer {
    fn size(&self) -> usize {
        self.object.size()
    }
}

impl Mappable for DrmPrimeBuffer {
    fn map(&self, offset: usize, handle: MapHandle) -> Result<Box<dyn MappedObject>> {
        self.object.create_mapped_object(offset, handle.size())
    }
}
