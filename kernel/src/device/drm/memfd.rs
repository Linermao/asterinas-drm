use alloc::{string::ToString, sync::Arc};

use aster_gpu::drm::{DrmError, gem::DrmGemObject};
use ostd::mm::{VmReader, VmWriter};

use crate::fs::{
    file_handle::{FileLike, Mappable},
    inode_handle::InodeHandle,
    ramfs::memfd::{MemfdFlags, MemfdInodeHandle},
    utils::FallocMode,
};

#[derive(Debug)]
pub struct DrmMemFdFile {
    inode_handle: InodeHandle,
    pitch: u32,
    size: u64,
}

impl DrmMemFdFile {
    pub fn mappable(&self) -> crate::prelude::Result<Mappable> {
        self.inode_handle.mappable()
    }
}

impl DrmGemObject for DrmMemFdFile {
    fn pitch(&self) -> u32 {
        self.pitch
    }

    fn size(&self) -> u64 {
        self.size
    }

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
}

pub fn memfd_allocator(
    name: &str,
    pitch: u32,
    size: u64,
) -> Result<Arc<dyn DrmGemObject>, DrmError> {
    let memfd = InodeHandle::new_memfd(name.to_string(), MemfdFlags::MFD_ALLOW_SEALING)
        .map_err(|_| DrmError::Invalid)?;

    memfd
        .fallocate(FallocMode::Allocate, 0, size as usize)
        .map_err(|_| DrmError::Invalid)?;

    Ok(Arc::new(DrmMemFdFile {
        pitch,
        size,
        inode_handle: memfd,
    }))
}
