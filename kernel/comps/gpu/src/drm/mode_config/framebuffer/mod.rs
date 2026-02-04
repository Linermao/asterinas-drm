use alloc::{boxed::Box, sync::Arc};

use hashbrown::HashMap;
use ostd::mm::{VmReader, VmWriter};

use crate::drm::{
    DrmError,
    gem::DrmGemObject,
    mode_config::{DrmModeObject, framebuffer::funcs::FramebufferFuncs},
};

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
    funcs: Box<dyn FramebufferFuncs>,
}

impl DrmFramebuffer {
    pub fn new(
        width: u32,
        height: u32,
        pitch: u32,
        bpp: u32,
        gem_obj: Arc<DrmGemObject>,
        funcs: Box<dyn FramebufferFuncs>,
    ) -> Self {
        Self {
            id: 0,
            width,
            height,
            pitch,
            bpp,
            gem_obj,

            properties: HashMap::new(),
            funcs,
        }
    }

    pub fn init_object(&mut self, id: u32) {
        self.id = id
    }

    pub fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<usize, DrmError> {
        self.gem_obj.read(offset, writer)
    }

    pub fn write(&self, offset: usize, reader: &mut VmReader) -> Result<usize, DrmError> {
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
