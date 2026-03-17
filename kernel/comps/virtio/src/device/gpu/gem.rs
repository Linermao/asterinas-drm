use alloc::boxed::Box;

use aster_gpu::drm::gem::{DrmGemBackend, DrmGemObject};

#[derive(Debug)]
pub struct VirtioGemObject {
    backend: Box<dyn DrmGemBackend>,
    resource_id: u32,

    pitch: u32,
    size: u64,
}

impl VirtioGemObject {
    pub fn new(pitch: u32, size: u64, backend: Box<dyn DrmGemBackend>, resource_id: u32) -> Self {
        Self {
            backend,
            resource_id,
            pitch,
            size,
        }
    }

    pub fn resource_id(&self) -> u32 {
        self.resource_id
    }
}

impl DrmGemObject for VirtioGemObject {
    fn backend(&self) -> &Box<dyn DrmGemBackend> {
        &self.backend
    }
    
    fn pitch(&self) -> u32 {
        self.pitch
    }
    
    fn size(&self) -> u64 {
        self.size
    }
}