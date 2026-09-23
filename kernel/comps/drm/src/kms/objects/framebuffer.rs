// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::fmt::Debug;

use aster_core::prelude::*;

use crate::{gem::object::DrmGemObject, utils::DrmDisplayFormat};

pub const DRM_FORMAT_MAX_PLANES: usize = 4;

#[derive(Debug)]
pub struct DrmFramebuffer {
    width: u32,
    height: u32,
    pixel_format: DrmDisplayFormat,
    flags: u32,
    pitches: u32,
    offsets: u32,
    modifiers: u64,
    gem_objects: Option<Arc<dyn DrmGemObject>>,
}

impl DrmFramebuffer {
    pub fn new(
        width: u32,
        height: u32,
        pixel_format: DrmDisplayFormat,
        flags: u32,
        pitches: u32,
        offsets: u32,
        modifiers: u64,
        gem_objects: Option<Arc<dyn DrmGemObject>>,
    ) -> Result<Self> {
        if width == 0 || height == 0 || pitches == 0 || gem_objects.is_none() {
            return_errno_with_message!(
                Errno::EINVAL,
                "the DRM framebuffer dimensions, pitch, and GEM object must be valid"
            );
        }

        Ok(Self {
            width,
            height,
            pixel_format,
            flags,
            pitches,
            offsets,
            modifiers,
            gem_objects,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixel_format(&self) -> DrmDisplayFormat {
        self.pixel_format
    }

    pub fn flags(&self) -> u32 {
        self.flags
    }

    pub fn pitches(&self) -> u32 {
        self.pitches
    }

    pub fn offsets(&self) -> u32 {
        self.offsets
    }

    pub fn modifiers(&self) -> u64 {
        self.modifiers
    }

    pub fn gem_object(&self) -> Option<&Arc<dyn DrmGemObject>> {
        self.gem_objects.as_ref()
    }
}
