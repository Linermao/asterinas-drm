use alloc::{format, sync::Arc};

use ostd::mm::{VmReader, VmWriter};

use crate::{
    device::drm::{
        driver::DrmDriverOps,
        gem::{DrmGemBackend, DrmGemObject},
    },
    fs::{
        file_handle::{FileLike, Mappable},
        ramfs::memfd::{MemfdFile, MemfdFlags},
        utils::FallocMode,
    },
    prelude::*,
};

/// This type wraps a `MemfdFile` as a GEM buffer backend suitable for
/// drivers that use GEM to manage buffer objects. In Linux DRM, GEM
/// objects are abstract buffer objects backed by anonymous memory (often
/// via the shmem filesystem) that drivers expose to userspace for scanout
/// and other operations. 
/// 
/// `DrmMemfdFile` implements the `DrmGemBackend` trait, providing
/// read/write and release callbacks to satisfy GEM’s buffer operations.
/// This can be used in simple or virtual drivers where a generic, 
/// pageable memory backend is sufficient (similar in role to a shmem
/// GEM object). It is analogous to Linux drivers using `drm_gem_object_init`
/// with shmem backing, with memfd representing the underlying file.
#[derive(Debug)]
pub struct DrmMemfdFile(MemfdFile);

impl DrmMemfdFile {
    pub fn new(name: &str, size: usize) -> Result<Arc<dyn DrmGemBackend>> {
        let name = format!("/gem:{}", name);
        let memfd = MemfdFile::new(&name, MemfdFlags::MFD_ALLOW_SEALING)?;
        memfd.fallocate(FallocMode::Allocate, 0, size)?;
        Ok(Arc::new(DrmMemfdFile(memfd)))
    }

    pub fn mappable(&self) -> Result<Mappable> {
        self.0.mappable()
    }
}

impl DrmGemBackend for DrmMemfdFile {
    fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        self.0.read_at(offset, writer)
    }

    fn write(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        self.0.write_at(offset, reader)
    }

    fn release(&self) -> Result<()> {
        self.0.resize(0)
    }
}

struct DrmMemfdDriverOps;

impl DrmMemfdDriverOps {
    fn dumb_create_impl(width: u32, height: u32, bpp: u32) -> Result<Arc<DrmGemObject>> {
        let pitch = width * (bpp / 8);
        let size = pitch * height;

        let backend = DrmMemfdFile::new("some", size as usize)?;
        let gem_object = DrmGemObject::new(size as u64, pitch, backend);
        Ok(Arc::new(gem_object))
    }
}

pub const DRM_MEMFD_DRIVER_OPS: DrmDriverOps = DrmDriverOps {
    dumb_create: Some(DrmMemfdDriverOps::dumb_create_impl),
};
