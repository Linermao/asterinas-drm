use alloc::sync::Arc;

use hashbrown::HashMap;
use ostd::mm::{VmReader, VmWriter};

use crate::drm::{gem::DrmGemObject, mode_config::DrmModeObject};

pub mod funcs;
pub mod property;

#[derive(Debug)]
pub struct DrmFramebuffer {
    id: u32,
    width: u32,
    height: u32,
    pitch: u32,
    bpp: u32,
    gem_obj: Arc<DrmGemObject>,

    properties: HashMap<u32, u64>,
}

impl DrmFramebuffer {
    pub fn new(
        id: u32,
        width: u32,
        height: u32,
        pitch: u32,
        bpp: u32,
        gem_obj: Arc<DrmGemObject>,
    ) -> Self {
        Self {
            id,
            width,
            height,
            pitch,
            bpp,
            gem_obj,

            properties: HashMap::new(),
        }
    }

    pub fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<usize, ()> {
        self.gem_obj.read(offset, writer)
    }

    pub fn write(&self, offset: usize, reader: &mut VmReader) -> Result<usize, ()> {
        self.gem_obj.write(offset, reader)
    }
}

impl DrmModeObject for DrmFramebuffer {
    fn id(&self) -> u32 {
        self.id
    }

    fn properties(&self) -> &HashMap<u32, u64> {
        &self.properties
    }
}
