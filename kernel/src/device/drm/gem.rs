// SPDX-License-Identifier: MPL-2.0

use aster_drm::{DrmError, DrmGemMapPage, DrmGemObject, DrmSgEntry, DrmVmaOffsetNode};
use ostd::mm::HasPaddr;

use crate::{
    prelude::*,
    vm::page_cache::{Vmo, VmoFlags, VmoOptions},
};

#[derive(Debug)]
pub(super) struct DrmGemShmemObject {
    vmo: Arc<Vmo>,
    size: usize,
    pitch: u32,
    vma_node: Arc<DrmVmaOffsetNode>,
}

impl DrmGemShmemObject {
    pub fn new(size: usize, pitch: u32) -> core::result::Result<Self, DrmError> {
        let vmo = VmoOptions::new(size)
            .flags(VmoFlags::RESIZABLE)
            .alloc()
            .map_err(|_| DrmError::NoMemory)?;

        let vma_node = Arc::new(DrmVmaOffsetNode::new());

        Ok(Self {
            vmo,
            size,
            pitch,
            vma_node,
        })
    }
}

impl DrmGemObject for DrmGemShmemObject {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn read(&self, offset: usize, writer: &mut VmWriter) -> core::result::Result<(), DrmError> {
        self.vmo.read(offset, writer).map_err(|_| DrmError::Invalid)
    }

    fn write(&self, offset: usize, reader: &mut VmReader) -> core::result::Result<(), DrmError> {
        self.vmo
            .write(offset, reader)
            .map_err(|_| DrmError::Invalid)
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

    fn sg_entries(&self) -> core::result::Result<Vec<DrmSgEntry>, DrmError> {
        let mut entries: Vec<DrmSgEntry> = Vec::new();
        let size = self.size();

        for offset in (0..size).step_by(PAGE_SIZE) {
            let page_len = (size - offset).min(PAGE_SIZE);
            let DrmGemMapPage::Frame(frame) = self.map_page(offset)? else {
                return Err(DrmError::Invalid);
            };

            let addr = u64::try_from(frame.paddr()).map_err(|_| DrmError::Invalid)?;
            let length = u32::try_from(page_len).map_err(|_| DrmError::Invalid)?;

            if let Some(last_entry) = entries.last_mut() {
                let last_end = last_entry
                    .addr()
                    .checked_add(u64::from(last_entry.length()))
                    .ok_or(DrmError::Invalid)?;
                let merged_length = last_entry
                    .length()
                    .checked_add(length)
                    .ok_or(DrmError::Invalid)?;

                if last_end == addr {
                    last_entry.update_length(merged_length);
                    continue;
                }
            }

            entries.push(DrmSgEntry::new(addr, length));
        }

        Ok(entries)
    }

    fn map_page(&self, offset: usize) -> core::result::Result<DrmGemMapPage, DrmError> {
        if offset % PAGE_SIZE != 0 {
            return Err(DrmError::Invalid);
        }

        let mapped_size = self
            .size
            .div_ceil(PAGE_SIZE)
            .checked_mul(PAGE_SIZE)
            .ok_or(DrmError::NoMemory)?;
        if offset >= mapped_size {
            return Err(DrmError::Invalid);
        }

        let page_idx = offset / PAGE_SIZE;
        let frame = self
            .vmo
            .commit_on(page_idx)
            .map_err(|error| match error.error() {
                Errno::EINVAL => DrmError::Invalid,
                Errno::ENOMEM => DrmError::NoMemory,
                Errno::EBUSY | Errno::EIO => DrmError::Busy,
                _ => DrmError::NoMemory,
            })?;
        Ok(DrmGemMapPage::Frame(frame.into()))
    }
}
