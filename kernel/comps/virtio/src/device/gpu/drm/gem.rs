use alloc::sync::Arc;

use aster_gpu::drm::{
    DrmError, driver::MemfdBackendCreateFunc, gem::DrmGemObject, ioctl::DrmModeCreateDumb,
};

pub struct VirtioGpuObject {
    gem_object: Arc<DrmGemObject>,
    hw_res_handle: u32,
    dumb: bool,
    created: bool,
    attached: bool,
    host3d_blob: bool,
    guest_blob: bool,
    blob_mem: u32,
    blob_flags: u32,
}

impl VirtioGpuObject {
    fn new(gem_object: Arc<DrmGemObject>) -> Self {
        Self {
            gem_object,
            hw_res_handle: 0,
            dumb: false,
            created: false,
            attached: false,
            host3d_blob: false,
            guest_blob: false,
            blob_mem: 0,
            blob_flags: 0,
        }
    }
}

pub fn virtio_gpu_object_create(
    size: u64,
    pitch: u32,
    memfd_backend_create: MemfdBackendCreateFunc,
) -> Result<Arc<DrmGemObject>, DrmError> {
    // need shmem
    let backend = memfd_backend_create("virtio-gpu-dumb", size)?;
    let gem_object = Arc::new(DrmGemObject::new(size, pitch, backend));

    let _virtio_gpu_obj = VirtioGpuObject::new(gem_object.clone());

    Ok(gem_object)
}

pub fn virtio_gpu_mode_dumb_create(
    args: &mut DrmModeCreateDumb,
    memfd_backend_create: MemfdBackendCreateFunc,
) -> Result<Arc<DrmGemObject>, DrmError> {
    if args.bpp != 32 {
        return Err(DrmError::Invalid);
    }

    let pitch = args.width * 4;
    let size = (pitch * args.height) as u64;

    args.pitch = pitch;
    args.size = size;

    virtio_gpu_object_create(size, pitch, memfd_backend_create)
}
