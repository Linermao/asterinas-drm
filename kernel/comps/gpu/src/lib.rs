// SPDX-License-Identifier: MPL-2.0

#![no_std]
#![deny(unsafe_code)]

use alloc::{sync::Arc, vec::Vec};

use component::{ComponentInitError, init_component};
use ostd::sync::Mutex;
use spin::Once;

use crate::drm::DrmDevice;

extern crate alloc;
#[macro_use]
extern crate ostd_pod;

pub mod drm;

/// Error type for GPU device registry operations.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    AlreadyRegistered,
    NotFound,
}

impl From<Error> for ComponentInitError {
    fn from(error: Error) -> Self {
        match error {
            Error::AlreadyRegistered => {
                log::warn!("[kernel] DRM: The device already registered")
            }
            _ => {}
        }
        ComponentInitError::Unknown
    }
}

/// Registers a GPU device.
pub fn register_device(device: Arc<dyn DrmDevice>) -> Result<(), Error> {
    let component = COMPONENT
        .get()
        .expect("aster-gpu component not initialized");

    component.gpu_devices.lock().push(device);

    Ok(())
}

/// Registers a GPU device.
pub fn registered_devices() -> Vec<Arc<dyn DrmDevice>> {
    let component = COMPONENT
        .get()
        .expect("aster-gpu component not initialized");

    component.gpu_devices.lock().clone()
}

/// Unregisters a GPU device.
pub fn unregister_device(device: &Arc<dyn DrmDevice>) -> Result<Arc<dyn DrmDevice>, Error> {
    let component = COMPONENT
        .get()
        .expect("aster-gpu component not initialized");

    let mut devices = component.gpu_devices.lock();
    if let Some(pos) = devices.iter().position(|d| Arc::ptr_eq(d, device)) {
        Ok(devices.remove(pos))
    } else {
        Err(Error::NotFound)
    }
}

static COMPONENT: Once<Component> = Once::new();

#[init_component]
fn component_init() -> Result<(), ComponentInitError> {
    let component = Component::init()?;
    COMPONENT.call_once(|| component);
    Ok(())
}

#[derive(Debug)]
struct Component {
    gpu_devices: Mutex<Vec<Arc<dyn DrmDevice>>>,
}

impl Component {
    fn init() -> Result<Self, ComponentInitError> {
        Ok(Self {
            gpu_devices: Mutex::new(Vec::new()),
        })
    }
}
