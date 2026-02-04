use alloc::{format, sync::Arc};

use aster_gpu::drm::{
    DrmError,
    gem::{DrmGemBackend, DrmGemObject},
};
use ostd::mm::{VmReader, VmWriter};

use crate::fs::{
    file_handle::{FileLike, Mappable},
    ramfs::memfd::{MemfdFile, MemfdFlags},
    utils::FallocMode,
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
    pub fn new(name: &str, size: usize) -> crate::prelude::Result<Arc<dyn DrmGemBackend>> {
        let name = format!("/gem:{}", name);
        let memfd = MemfdFile::new(&name, MemfdFlags::MFD_ALLOW_SEALING)?;
        memfd.fallocate(FallocMode::Allocate, 0, size)?;
        Ok(Arc::new(DrmMemfdFile(memfd)))
    }

    pub fn mappable(&self) -> crate::prelude::Result<Mappable> {
        self.0.mappable()
    }
}

// TODO: How to convert Error to DrmError? Is this nesseary?
impl DrmGemBackend for DrmMemfdFile {
    fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<usize, DrmError> {
        self.0
            .read_at(offset, writer)
            .map_err(|_| DrmError::Invalid)
    }

    fn write(&self, offset: usize, reader: &mut VmReader) -> Result<usize, DrmError> {
        self.0
            .write_at(offset, reader)
            .map_err(|_| DrmError::Invalid)
    }

    fn release(&self) -> Result<(), DrmError> {
        self.0.resize(0).map_err(|_| DrmError::Invalid)
    }
}

pub fn dumb_create_impl(
    width: u32,
    height: u32,
    bpp: u32,
) -> crate::prelude::Result<Arc<DrmGemObject>> {
    let pitch = width * (bpp / 8);
    let size = pitch * height;

    // TODO: handle the error
    let backend = DrmMemfdFile::new("some", size as usize)?;
    let gem_object = DrmGemObject::new(size as u64, pitch, backend);
    Ok(Arc::new(gem_object))
}
