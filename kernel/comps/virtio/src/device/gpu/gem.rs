// SPDX-License-Identifier: MPL-2.0

use alloc::{
    collections::BTreeMap,
    sync::{Arc, Weak},
    vec::Vec,
};
use core::{fmt, ops::Range, time::Duration};

use aster_drm::{DrmError, DrmFence, DrmGemMapPage, DrmGemObject, DrmSgEntry, DrmVmaOffsetNode};
use ostd::{
    io::IoMem,
    mm::{HasSize, PAGE_SIZE, VmIo, VmReader, VmWriter},
    sync::Mutex,
};

use super::{device::GpuDevice, ioctl::VirtioGpuBlobMemFlags};
use crate::device::gpu::queue::header::{VirtioGpuResourceUnmapBlob, VirtioGpuResourceUnref};

/// Manages allocations in the virtio-gpu host-visible shared-memory aperture.
pub(super) struct VirtioGpuHostVisibleMemory {
    memory: IoMem,
    free_ranges: Mutex<BTreeMap<usize, usize>>,
    size: usize,
}

impl VirtioGpuHostVisibleMemory {
    pub(super) fn new(memory: IoMem, length: u64) -> Result<Arc<Self>, DrmError> {
        let length = usize::try_from(length).map_err(|_| DrmError::Invalid)?;
        let size = length.min(memory.size()) / PAGE_SIZE * PAGE_SIZE;
        if size == 0 {
            return Err(DrmError::Invalid);
        }

        let mut free_ranges = BTreeMap::new();
        free_ranges.insert(0, size);

        Ok(Arc::new(Self {
            memory: memory.slice(0..size),
            free_ranges: Mutex::new(free_ranges),
            size,
        }))
    }
    pub(super) fn allocate(
        self: &Arc<Self>,
        size: usize,
    ) -> Result<VirtioGpuHostVisibleAllocation, DrmError> {
        if size == 0 || !size.is_multiple_of(PAGE_SIZE) {
            return Err(DrmError::Invalid);
        }

        let mut free_ranges = self.free_ranges.lock();
        let (start, available) = free_ranges
            .iter()
            .find_map(|(&start, &available)| (available >= size).then_some((start, available)))
            .ok_or(DrmError::NoMemory)?;
        let end = start.checked_add(size).ok_or(DrmError::NoMemory)?;
        free_ranges.remove(&start);
        if available > size {
            free_ranges.insert(end, available - size);
        }
        drop(free_ranges);

        let range = start..end;
        Ok(VirtioGpuHostVisibleAllocation {
            owner: self.clone(),
            range,
        })
    }

    fn free(&self, range: Range<usize>) {
        debug_assert!(!range.is_empty() && range.end <= self.size);

        let mut free_ranges = self.free_ranges.lock();
        let mut start = range.start;
        let mut end = range.end;

        if let Some((&previous_start, &previous_size)) = free_ranges.range(..start).next_back()
            && previous_start.checked_add(previous_size) == Some(start)
        {
            start = previous_start;
            free_ranges.remove(&previous_start);
        }

        if let Some((&next_start, &next_size)) = free_ranges.range(end..).next()
            && end == next_start
        {
            let next_end = next_start.saturating_add(next_size);
            debug_assert!(next_end <= self.size);
            end = next_end;
            free_ranges.remove(&next_start);
        }

        let previous = free_ranges.insert(start, end - start);
        debug_assert!(previous.is_none());
    }
}

impl fmt::Debug for VirtioGpuHostVisibleMemory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VirtioGpuHostVisibleMemory")
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub(super) struct VirtioGpuHostVisibleAllocation {
    owner: Arc<VirtioGpuHostVisibleMemory>,
    range: Range<usize>,
}

impl VirtioGpuHostVisibleAllocation {
    pub(super) fn offset(&self) -> usize {
        self.range.start
    }

    fn size(&self) -> usize {
        self.range.len()
    }

    fn page(&self, offset: usize) -> Result<IoMem, DrmError> {
        if !offset.is_multiple_of(PAGE_SIZE) {
            return Err(DrmError::Invalid);
        }

        let page_end = offset.checked_add(PAGE_SIZE).ok_or(DrmError::Invalid)?;
        if page_end > self.size() {
            return Err(DrmError::Invalid);
        }

        let page_start = self
            .range
            .start
            .checked_add(offset)
            .ok_or(DrmError::Invalid)?;
        let page_end = page_start.checked_add(PAGE_SIZE).ok_or(DrmError::Invalid)?;
        Ok(self.owner.memory.slice(page_start..page_end))
    }

    fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<(), DrmError> {
        let end = offset
            .checked_add(writer.avail())
            .ok_or(DrmError::Invalid)?;
        if end > self.size() {
            return Err(DrmError::Invalid);
        }

        let memory_offset = self
            .range
            .start
            .checked_add(offset)
            .ok_or(DrmError::Invalid)?;
        self.owner
            .memory
            .read(memory_offset, writer)
            .map_err(|_| DrmError::Invalid)
    }

    fn write(&self, offset: usize, reader: &mut VmReader) -> Result<(), DrmError> {
        let end = offset
            .checked_add(reader.remain())
            .ok_or(DrmError::Invalid)?;
        if end > self.size() {
            return Err(DrmError::Invalid);
        }

        let memory_offset = self
            .range
            .start
            .checked_add(offset)
            .ok_or(DrmError::Invalid)?;
        self.owner
            .memory
            .write(memory_offset, reader)
            .map_err(|_| DrmError::Invalid)
    }
}

impl Drop for VirtioGpuHostVisibleAllocation {
    fn drop(&mut self) {
        self.owner.free(self.range.clone());
    }
}

/// Provides GEM mmap behavior for a host-only blob resource.
#[derive(Debug)]
pub(super) struct VirtioGpuHostBlobObject {
    size: usize,
    mapping: Mutex<Option<VirtioGpuHostVisibleAllocation>>,
    vma_node: Arc<DrmVmaOffsetNode>,
}

impl VirtioGpuHostBlobObject {
    pub(super) fn new(
        size: usize,
        mapping: Option<VirtioGpuHostVisibleAllocation>,
    ) -> Result<Self, DrmError> {
        if size == 0 || !size.is_multiple_of(PAGE_SIZE) {
            return Err(DrmError::Invalid);
        }
        if mapping
            .as_ref()
            .is_some_and(|mapping| mapping.size() < size)
        {
            return Err(DrmError::Invalid);
        }

        Ok(Self {
            size,
            mapping: Mutex::new(mapping),
            vma_node: Arc::new(DrmVmaOffsetNode::new()),
        })
    }

    pub(super) fn take_mapping(&self) -> Option<VirtioGpuHostVisibleAllocation> {
        self.mapping.lock().take()
    }
}

impl DrmGemObject for VirtioGpuHostBlobObject {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<(), DrmError> {
        let mapping = self.mapping.lock();
        mapping
            .as_ref()
            .ok_or(DrmError::Invalid)?
            .read(offset, writer)
    }

    fn write(&self, offset: usize, reader: &mut VmReader) -> Result<(), DrmError> {
        let mapping = self.mapping.lock();
        mapping
            .as_ref()
            .ok_or(DrmError::Invalid)?
            .write(offset, reader)
    }

    fn size(&self) -> usize {
        self.size
    }

    fn pitch(&self) -> u32 {
        0
    }

    fn vma_node(&self) -> &Arc<DrmVmaOffsetNode> {
        &self.vma_node
    }

    fn sg_entries(&self) -> Result<Vec<DrmSgEntry>, DrmError> {
        // Host-only blobs have no guest backing entries.
        Ok(Vec::new())
    }

    fn map_page(&self, offset: usize) -> Result<DrmGemMapPage, DrmError> {
        let mapping = self.mapping.lock();
        let mapping = mapping.as_ref().ok_or(DrmError::Invalid)?;
        Ok(DrmGemMapPage::IoMem(mapping.page(offset)?))
    }
}

#[derive(Debug)]
pub(super) struct VirtioGpuGemObject {
    device: Weak<GpuDevice>,
    is_3d: bool,
    inner: Arc<dyn DrmGemObject>,
    resource_id: u32,
    blob_mem: Option<VirtioGpuBlobMemFlags>,
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
        Self::new_inner(device, is_3d, inner, resource_id, None, fence)
    }

    pub(super) fn new_blob(
        device: Weak<GpuDevice>,
        is_3d: bool,
        inner: Arc<dyn DrmGemObject>,
        resource_id: u32,
        blob_mem: VirtioGpuBlobMemFlags,
        fence: Option<Arc<DrmFence>>,
    ) -> Self {
        Self::new_inner(device, is_3d, inner, resource_id, Some(blob_mem), fence)
    }

    fn new_inner(
        device: Weak<GpuDevice>,
        is_3d: bool,
        inner: Arc<dyn DrmGemObject>,
        resource_id: u32,
        blob_mem: Option<VirtioGpuBlobMemFlags>,
        fence: Option<Arc<DrmFence>>,
    ) -> Self {
        Self {
            device,
            is_3d,
            inner,
            resource_id,
            blob_mem,
            pending_fences: Mutex::new(fence.into_iter().collect()),
        }
    }

    pub(super) fn is_3d(&self) -> bool {
        self.is_3d
    }

    pub(super) fn resource_id(&self) -> u32 {
        self.resource_id
    }

    pub(super) fn blob_mem(&self) -> Option<VirtioGpuBlobMemFlags> {
        self.blob_mem
    }

    pub(super) fn is_guest_only_blob(&self) -> bool {
        matches!(self.blob_mem(), Some(VirtioGpuBlobMemFlags::Guest))
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

        let host_mapping = self
            .inner
            .as_any()
            .downcast_ref::<VirtioGpuHostBlobObject>()
            .and_then(VirtioGpuHostBlobObject::take_mapping);
        if let Some(host_mapping) = host_mapping {
            let request = VirtioGpuResourceUnmapBlob::new(self.resource_id);
            if let Err(err) = device.unmap_blob_resource(request) {
                // TODO: Retain failed cleanup operations in a normal task-context
                // worker. Releasing this aperture range before UNMAP is queued
                // could alias a still-mapped host resource, while retaining it in
                // the IRQ bottom half could run sleeping cleanup from Taskless.
                core::mem::forget(host_mapping);
                ostd::warn!(
                    "virtio-gpu failed to unmap blob resource {} on GEM drop: {:?}",
                    self.resource_id,
                    err
                );
            } else {
                // The range may be reused once UNMAP is ordered on controlq; a
                // later MAP using the same range is necessarily queued after it.
                drop(host_mapping);
            }
        }

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
