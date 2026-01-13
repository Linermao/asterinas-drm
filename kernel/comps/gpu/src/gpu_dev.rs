// SPDX-License-Identifier: MPL-2.0

use alloc::{sync::Arc, vec::Vec};
use core::{any::Any, fmt::Debug};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceError {
    Errno(i32),
}

/// A low-level abstraction representing a GPU-capable device that has been
/// discovered by the system, but is not yet bound to any DRM driver.
///
/// `GpuDevice` is implemented by bus- or platform-specific device objects
/// (e.g. PCI, Virtio, platform firmware devices) to advertise that the device
/// provides GPU functionality.
///
/// The purpose of this trait is **driver matching and probing**, not device
/// lifetime management or DRM node representation.
///
/// Typical flow:
/// 1. A concrete device (e.g. `VirtioGpuDevice`) is discovered by its bus.
/// 2. The device implements `GpuDevice` trait to declare GPU capability.
/// 3. The DRM core selects a compatible `DrmDriver` based on device properties.
/// 4. One or more `DrmDevice` instances are created and bound to the driver.
///
/// Note:
/// - `GpuDevice` does NOT represent a DRM device node.
/// - `GpuDevice` does NOT handle char device registration or file operations.
/// - A single `GpuDevice` may result in multiple DRM nodes (primary/render/control).
/// 
/// Example:
/// ```rust
/// #[derive(Debug)]
/// struct SimpleGpuDevice;
/// 
/// impl GpuDevice for SimpleGpuDevice {
///     fn name(&self) -> &str {
///         SIMPLEDRM_NAME
///     }
/// }
/// 
/// pub fn init() {
///     let device = Arc::new(SimpleGpuDevice {});
///     aster_gpu::register_device(device).expect("failed to register simple_drm GpuDevice");
/// }
/// ```
pub trait GpuDevice: Send + Sync + Any + Debug {
    /// Human-readable device name, used for debugging, logging,
    /// and optional driver matching.
    fn name(&self) -> &str;
    // more settings e.g. device_id, capability, resources
}

#[derive(Debug, Default)]
pub(crate) struct GpuDevices {
    devices: Vec<Arc<dyn GpuDevice>>,
}

impl GpuDevices {
    pub fn new() -> Self {
        Self {
            devices: Vec::new(),
        }
    }

    /// Snapshot (clone Arcs) so caller can use it after unlocking the mutex.
    pub fn snapshot(&self) -> Vec<Arc<dyn GpuDevice>> {
        self.devices.clone()
    }

    pub fn register_device(&mut self, device: Arc<dyn GpuDevice>) -> Result<(), super::Error> {
        // TODO: Simple duplicate policy: add a stable device id and dedup by id.
        self.devices.push(device);
        Ok(())
    }

    pub fn unregister_device(
        &mut self,
        device: &Arc<dyn GpuDevice>,
    ) -> Result<Arc<dyn GpuDevice>, super::Error> {
        if let Some(pos) = self.devices.iter().position(|d| Arc::ptr_eq(d, device)) {
            Ok(self.devices.remove(pos))
        } else {
            Err(super::Error::NotFound)
        }
    }
}
