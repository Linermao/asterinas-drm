// SPDX-License-Identifier: MPL-2.0

#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

pub mod drm;
mod gpu_dev;

use alloc::{string::String, sync::Arc, vec::Vec};

use component::{ComponentInitError, init_component};
pub use gpu_dev::GpuDevice;
use hashbrown::HashMap;
use ostd::sync::Mutex;
use spin::Once;

use crate::{
    drm::{DrmDrivers, driver::DrmDriver},
    gpu_dev::GpuDevices,
};

/// Error type for GPU device registry operations.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    AlreadyRegistered,
    NotFound,
}

/// Registers a DRM driver.
pub fn register_driver(name: &str, driver: Arc<dyn DrmDriver>) -> Result<(), Error> {
    let component = COMPONENT
        .get()
        .expect("aster-gpu component not initialized");
    component.drm_drivers.lock().register_driver(name, driver)
}

/// Unregisters a DRM driver.
pub fn unregister_driver(name: &str) -> Result<Arc<dyn DrmDriver>, Error> {
    let component = COMPONENT
        .get()
        .expect("aster-gpu component not initialized");
    component.drm_drivers.lock().unregister_driver(name)
}

/// Returns a snapshot of all registered GPU drivers.
pub fn registered_drivers() -> HashMap<String, Arc<dyn DrmDriver>> {
    let component = COMPONENT
        .get()
        .expect("aster-gpu component not initialized");
    component.drm_drivers.lock().snapshot()
}

/// Registers a GPU device.
pub fn register_device(device: Arc<dyn GpuDevice>) -> Result<(), Error> {
    let component = COMPONENT
        .get()
        .expect("aster-gpu component not initialized");
    component.gpu_devices.lock().register_device(device)
}

/// Unregisters a GPU device.
pub fn unregister_device(device: &Arc<dyn GpuDevice>) -> Result<Arc<dyn GpuDevice>, Error> {
    let component = COMPONENT
        .get()
        .expect("aster-gpu component not initialized");
    component.gpu_devices.lock().unregister_device(device)
}

/// Returns a snapshot of all registered GPU devices.
pub fn registered_devices() -> Vec<Arc<dyn GpuDevice>> {
    let component = COMPONENT
        .get()
        .expect("aster-gpu component not initialized");
    component.gpu_devices.lock().snapshot()
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
    gpu_devices: Mutex<GpuDevices>,
    drm_drivers: Mutex<DrmDrivers>,
}

impl Component {
    fn init() -> Result<Self, ComponentInitError> {
        Ok(Self {
            gpu_devices: Mutex::new(GpuDevices::new()),
            drm_drivers: Mutex::new(DrmDrivers::new()),
        })
    }
}
