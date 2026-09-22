// SPDX-License-Identifier: MPL-2.0

use alloc::{sync::Arc, vec::Vec};

use aster_core::prelude::*;
use ostd::mm::{
    FallibleVmRead, FallibleVmWrite, FrameAllocOptions, PAGE_SIZE, UFrame, VmReader, VmWriter,
    io::util::HasVmReaderWriter,
};

use crate::gem::{
    object::{DrmGemMapPage, DrmGemObject},
    vma_manager::DrmVmaOffsetNode,
};

#[derive(Debug)]
pub struct DrmGemShmemObject {
    pages: Vec<UFrame>,
    pitch: u32,
    size: usize,
    vma_node: Arc<DrmVmaOffsetNode>,
}

impl DrmGemShmemObject {
    pub fn new(pitch: u32, size: usize) -> Result<Self> {
        if size == 0 {
            return_errno_with_message!(Errno::EINVAL, "the GEM object size must not be zero");
        }

        let Some(size) = size
            .checked_add(PAGE_SIZE - 1)
            .map(|size| size / PAGE_SIZE)
            .and_then(|num_pages| num_pages.checked_mul(PAGE_SIZE))
        else {
            return_errno_with_message!(Errno::ENOMEM, "the page-aligned GEM object size overflows");
        };
        let num_pages = size / PAGE_SIZE;

        // TODO: Anonymous VMOs currently do not expose the committed `UFrame` needed by
        // `MapHandle::map_frame`. Allocate all backing pages here so GEM mappings
        // can use the public OSTD interfaces without changing `aster-core`. This
        // makes creation proportional to the buffer size; once a suitable VMO or
        // sparse-frame interface is available, this should become demand-paged.
        let mut pages = Vec::<UFrame>::new();
        if pages.try_reserve_exact(num_pages).is_err() {
            return_errno_with_message!(Errno::ENOMEM, "failed to reserve GEM page handles");
        }
        for _ in 0..num_pages {
            pages.push(FrameAllocOptions::new().alloc_frame()?.into());
        }

        let vma_node = Arc::new(DrmVmaOffsetNode::new());

        Ok(Self {
            pages,
            pitch,
            size,
            vma_node,
        })
    }

    fn check_io_range(&self, offset: usize, len: usize) -> Result<()> {
        let Some(end) = offset.checked_add(len) else {
            return_errno_with_message!(Errno::EINVAL, "the GEM I/O range overflows");
        };
        if end > self.size {
            return_errno_with_message!(Errno::EINVAL, "the GEM I/O range is out of bounds");
        }

        Ok(())
    }
}

impl DrmGemObject for DrmGemShmemObject {
    fn read(&self, mut offset: usize, writer: &mut VmWriter) -> Result<()> {
        self.check_io_range(offset, writer.avail())?;

        while writer.has_avail() {
            let page_idx = offset / PAGE_SIZE;
            let page_offset = offset % PAGE_SIZE;
            let mut page_reader = self.pages[page_idx].reader();
            page_reader.skip(page_offset);
            let copied_len = page_reader.read_fallible(writer).map_err(|(err, _)| err)?;
            offset += copied_len;
        }

        Ok(())
    }

    fn write(&self, mut offset: usize, reader: &mut VmReader) -> Result<()> {
        self.check_io_range(offset, reader.remain())?;

        while reader.has_remain() {
            let page_idx = offset / PAGE_SIZE;
            let page_offset = offset % PAGE_SIZE;
            let mut page_writer = self.pages[page_idx].writer();
            page_writer.skip(page_offset);
            let copied_len = page_writer.write_fallible(reader).map_err(|(err, _)| err)?;
            offset += copied_len;
        }

        Ok(())
    }

    fn size(&self) -> usize {
        self.size
    }

    fn pitch(&self) -> u32 {
        self.pitch
    }

    fn vma_node(&self) -> &Arc<DrmVmaOffsetNode> {
        &self.vma_node
    }

    fn map_page(&self, offset: usize) -> Result<DrmGemMapPage> {
        if !offset.is_multiple_of(PAGE_SIZE) {
            return_errno_with_message!(Errno::EINVAL, "the GEM page offset is not page-aligned");
        }

        if offset >= self.size {
            return_errno_with_message!(Errno::EINVAL, "the GEM page offset is out of bounds");
        }

        let page_idx = offset / PAGE_SIZE;
        Ok(DrmGemMapPage::Frame(self.pages[page_idx].clone()))
    }
}
