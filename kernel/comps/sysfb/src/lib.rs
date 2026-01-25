// SPDX-License-Identifier: MPL-2.0

#![no_std]
#![deny(unsafe_code)]

mod simpledrm;

use component::{ComponentInitError, init_component};

extern crate alloc;

#[init_component]
fn sysfb_component_init() -> Result<(), ComponentInitError> {
    simpledrm::init();
    Ok(())
}
