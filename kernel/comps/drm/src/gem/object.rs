// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, sync::Arc};
use core::{any::Any, fmt::Debug};

use aster_core::{fs::file::MappedObject, prelude::*, vm::vmar::MapHandle};
use ostd::{
    io::IoMem,
    mm::{PAGE_SIZE, UFrame, VmReader, VmWriter},
};

use crate::gem::vma_manager::DrmVmaOffsetNode;

pub trait DrmGemObject: Any + Debug + Sync + Send {
    fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<()>;
    fn write(&self, offset: usize, reader: &mut VmReader) -> Result<()>;
    fn size(&self) -> usize;
    fn pitch(&self) -> u32;
    fn vma_node(&self) -> &Arc<DrmVmaOffsetNode>;
    /// Returns the page backing the page-aligned object-relative byte offset.
    fn map_page(&self, offset: usize) -> Result<DrmGemMapPage>;
}

#[derive(Debug)]
pub enum DrmGemMapPage {
    /// Maps one page of regular memory.
    Frame(UFrame),
    /// Maps one page of device I/O memory.
    IoMem(IoMem),
}

#[derive(Debug)]
pub struct DrmGemMappedObject {
    gem_object: Arc<dyn DrmGemObject>,
    object_offset: usize,
}

impl DrmGemMappedObject {
    pub fn new(gem_object: Arc<dyn DrmGemObject>, object_offset: usize) -> Self {
        Self {
            gem_object,
            object_offset,
        }
    }
}

impl MappedObject for DrmGemMappedObject {
    fn dup_at_offset(&self, offset: usize) -> Box<dyn MappedObject> {
        debug_assert!(offset.is_multiple_of(PAGE_SIZE));
        let object_offset = self.object_offset.saturating_add(offset);

        Box::new(Self::new(self.gem_object.clone(), object_offset))
    }

    fn handle_page_fault(&self, offset: usize, mut handle: MapHandle) -> Result<()> {
        debug_assert!(offset.is_multiple_of(PAGE_SIZE));
        let Some(object_offset) = self.object_offset.checked_add(offset) else {
            return_errno_with_message!(Errno::EFAULT, "the GEM page offset overflows");
        };
        if object_offset >= self.gem_object.size() {
            return_errno_with_message!(Errno::EFAULT, "the GEM page offset is out of bounds");
        }

        match self.gem_object.map_page(object_offset)? {
            DrmGemMapPage::Frame(frame) => handle.map_frame(offset, frame),
            DrmGemMapPage::IoMem(io_mem) => handle.map_iomem(offset, io_mem),
        }

        Ok(())
    }
}

/// A device mapping that represents an invalid DRM mmap request.
#[derive(Debug)]
pub(crate) struct DrmInvalidMappedObject;

impl MappedObject for DrmInvalidMappedObject {
    fn dup_at_offset(&self, _offset: usize) -> Box<dyn MappedObject> {
        Box::new(Self)
    }
}
