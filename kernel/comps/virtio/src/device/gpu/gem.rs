// SPDX-License-Identifier: MPL-2.0

use alloc::{
    sync::{Arc, Weak},
    vec::Vec,
};
use core::time::Duration;

use aster_drm::{DrmError, DrmFence, DrmGemMapPage, DrmGemObject, DrmSgEntry, DrmVmaOffsetNode};
use ostd::{
    mm::{VmReader, VmWriter},
    sync::Mutex,
};

use super::device::GpuDevice;
use crate::device::gpu::queue::header::VirtioGpuResourceUnref;

#[derive(Debug)]
pub(super) struct VirtioGpuGemObject {
    device: Weak<GpuDevice>,
    is_3d: bool,
    inner: Arc<dyn DrmGemObject>,
    resource_id: u32,
    pending_fences: Mutex<Vec<Arc<DrmFence>>>,
}

impl VirtioGpuGemObject {
    pub(super) fn new(
        device: Weak<GpuDevice>,
        is_3d: bool,
        inner: Arc<dyn DrmGemObject>,
        resource_id: u32,
        fence: Option<Arc<DrmFence>>,
    ) -> Self {
        Self {
            device,
            is_3d,
            inner,
            resource_id,
            pending_fences: Mutex::new(fence.into_iter().collect()),
        }
    }

    pub(super) fn is_3d(&self) -> bool {
        self.is_3d
    }

    pub(super) fn resource_id(&self) -> u32 {
        self.resource_id
    }

    pub(super) fn track_fence(&self, fence: Arc<DrmFence>) {
        let mut pending_fences = self.pending_fences.lock();
        pending_fences.retain(|pending_fence| !pending_fence.is_signaled());

        if !pending_fences
            .iter()
            .any(|pending_fence| Arc::ptr_eq(pending_fence, &fence))
        {
            pending_fences.push(fence);
        }
    }

    pub(super) fn has_pending_fences(&self) -> bool {
        !self.pending_fences().is_empty()
    }

    pub(super) fn wait_fences_timeout(&self, timeout: Option<Duration>) -> Result<(), DrmError> {
        for fence in self.pending_fences() {
            fence.wait_timeout(timeout)?;
        }

        Ok(())
    }

    fn pending_fences(&self) -> Vec<Arc<DrmFence>> {
        let mut pending_fences = self.pending_fences.lock();
        pending_fences.retain(|fence| !fence.is_signaled());
        pending_fences.clone()
    }
}

impl Drop for VirtioGpuGemObject {
    fn drop(&mut self) {
        let Some(device) = self.device.upgrade() else {
            return;
        };

        let request = VirtioGpuResourceUnref::new(self.resource_id);
        if let Err(err) = device.unref_resource(request, self.inner.clone()) {
            ostd::warn!(
                "virtio-gpu failed to unref resource {} on GEM drop: {:?}",
                self.resource_id,
                err
            );
        }
    }
}

impl DrmGemObject for VirtioGpuGemObject {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<(), DrmError> {
        self.inner.read(offset, writer)
    }

    fn write(&self, offset: usize, reader: &mut VmReader) -> Result<(), DrmError> {
        self.inner.write(offset, reader)
    }

    fn size(&self) -> usize {
        self.inner.size()
    }

    fn pitch(&self) -> u32 {
        self.inner.pitch()
    }

    fn vma_node(&self) -> &Arc<DrmVmaOffsetNode> {
        self.inner.vma_node()
    }

    fn sg_entries(&self) -> Result<Vec<DrmSgEntry>, DrmError> {
        self.inner.sg_entries()
    }

    fn map_page(&self, offset: usize) -> Result<DrmGemMapPage, DrmError> {
        self.inner.map_page(offset)
    }
}
