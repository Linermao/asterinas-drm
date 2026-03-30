use alloc::{boxed::Box, string::ToString, vec::Vec};

use aster_gpu::drm::{
    DrmError,
    gem::{DrmGemBackend, DrmSgEntry, DrmSgTable},
};
use ostd::mm::{HasPaddr, HasSize, PAGE_SIZE, VmReader, VmWriter};

use crate::{
    fs::{
        file_handle::{FileLike, Mappable},
        inode_handle::InodeHandle,
        ramfs::memfd::{MemfdFlags, MemfdInodeHandle},
        utils::FallocMode,
    },
    vm::vmo::CommitFlags,
};

#[derive(Debug)]
pub struct DrmMemFdFile {
    inode_handle: InodeHandle,
}

impl DrmMemFdFile {
    pub fn mappable(&self) -> crate::prelude::Result<Mappable> {
        self.inode_handle.mappable()
    }
}

impl DrmGemBackend for DrmMemFdFile {
    fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<usize, DrmError> {
        self.inode_handle
            .read_at(offset, writer)
            .map_err(|_| DrmError::Invalid)
    }

    fn write(&self, offset: usize, reader: &mut VmReader) -> Result<usize, DrmError> {
        self.inode_handle
            .write_at(offset, reader)
            .map_err(|_| DrmError::Invalid)
    }

    fn release(&self) -> Result<(), DrmError> {
        self.inode_handle.resize(0).map_err(|_| DrmError::Invalid)
    }

    fn get_pages_sgt(&self) -> Result<DrmSgTable, DrmError> {
        let vmo = self
            .inode_handle
            .path()
            .inode()
            .page_cache()
            .ok_or_else(|| DrmError::Invalid)?;

        let byte_sz = vmo.size();

        // `try_operate_on_range` works in **pages** not bytes. compute
        // number of pages to traverse instead of passing the raw byte
        // length, otherwise the iterator will iterate one entry per page
        // index (paging units are 4KiB) which explodes the SG length.
        let page_count = (byte_sz + PAGE_SIZE - 1) / PAGE_SIZE;
        // Collect physical addresses of each committed page and coalesce
        // contiguous pages into SG entries.
        let mut entries: Vec<DrmSgEntry> = Vec::new();
        for page_idx in 0..page_count {
            let frame = vmo
                .commit_on(page_idx, CommitFlags::empty())
                .map_err(|_| DrmError::Invalid)?;
            let p = frame.paddr();
            let s = frame.size();

            if let Some(last) = entries.last_mut() {
                let last_end = last.addr as usize + last.len as usize;
                if last_end == p {
                    last.len = last.len.saturating_add(s as u32);
                    continue;
                }
            }
            entries.push(DrmSgEntry {
                addr: p as u64,
                len: s as u32,
            });
        }
        
        Ok(DrmSgTable { entries })
    }
}

pub fn memfd_allocator_fn(
    name: &str,
    size: u64,
) -> Result<Box<dyn DrmGemBackend>, DrmError> {
    let memfd = InodeHandle::new_memfd(name.to_string(), MemfdFlags::MFD_ALLOW_SEALING)
        .map_err(|_| DrmError::Invalid)?;

    memfd
        .fallocate(FallocMode::Allocate, 0, size as usize)
        .map_err(|_| DrmError::Invalid)?;

    Ok(Box::new(DrmMemFdFile {
        inode_handle: memfd,
    }))
}
