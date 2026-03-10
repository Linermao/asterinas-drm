// SPDX-License-Identifier: MPL-2.0

#![no_std]
#![deny(unsafe_code)]

use alloc::sync::Arc;
use aster_framebuffer::FRAMEBUFFER;
use component::{ComponentInitError, init_component};

use crate::simpledrm::SimpleDrmDevice;

extern crate alloc;

pub mod simpledrm;

#[init_component]
fn sysfb_component_init() -> Result<(), ComponentInitError> {
    if FRAMEBUFFER.get().is_some() {
        let device = Arc::new(SimpleDrmDevice::new());
        aster_gpu::register_device(device)?;
    }
    Ok(())
}
