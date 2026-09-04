// SPDX-License-Identifier: MPL-2.0

//! The Direct Rendering Manager subsystem of Asterinas.

#![no_std]
#![deny(unsafe_code)]

use alloc::sync::Arc;

use aster_core::{
    device::{Device, char},
    prelude::*,
    process::{AsPosixThread, CapSet, UserNamespace},
    security::{self, CapableContext},
};
use ostd::{sync::Mutex, task::Task};
use sparse_id_alloc::SparseIdAlloc;

use crate::{
    device::{DrmDevice, DrmFeatures, RegisteredDrmDevice},
    minor::DrmMinor,
};

extern crate alloc;
#[macro_use]
extern crate ostd_pod;

// Set this crate's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "drm: "
    };
}

pub mod device;
mod file;
mod ioctl;
mod minor;
pub mod utils;

pub use file::{DrmFile, DrmFileCaps};
pub use minor::DrmMinorType;

fn has_current_sys_admin() -> bool {
    let Some(task) = Task::current() else {
        return false;
    };
    let Some(posix_thread) = task.as_posix_thread() else {
        return false;
    };

    security::on_capable(CapableContext::new(
        UserNamespace::get_init_singleton().as_ref(),
        posix_thread,
        CapSet::SYS_ADMIN,
    ))
    .is_ok()
}

// TODO: Reclaim an index after all of the device's minor nodes have been unregistered once DRM
// device unregistration is supported. For now, allocated indices remain reserved for the lifetime
// of the system because DRM devices cannot be unregistered.
static DRM_DEVICE_INDEX_ALLOCATOR: Mutex<SparseIdAlloc> = Mutex::new(SparseIdAlloc::new(0, 63));

pub fn register_device(driver: Arc<dyn DrmDevice>) -> Result<()> {
    let device = Arc::new(RegisteredDrmDevice::new(driver));
    let Some(index) = DRM_DEVICE_INDEX_ALLOCATOR.lock().alloc() else {
        return_errno_with_message!(Errno::ENOMEM, "no DRM device indices are available");
    };

    if device.has_feature(DrmFeatures::COMPUTE_ACCEL) {
        // TODO: Accel node (DRM_ACCEL) is intentionally not implemented for now.
        //
        // Rationale:
        // - The current DRM subsystem only targets primary (cardX) and render (renderDX) nodes.
        // - Modern userspace (Wayland/Mesa/Vulkan) does not rely on accel nodes.
        // - The accel minor is mainly used by specific compute-oriented drivers and is not
        //   required for virtio-gpu or basic KMS/render functionality.
        //
        // let drm_minor = DrmMinor::new(index, device.clone(), DrmMinorType::Accel);
        // char::register(drm_minor)?;
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "DRM acceleration devices are not supported"
        );
    }

    // TODO: Control node (controlD*) is intentionally not implemented.
    //
    // Rationale:
    // - The control minor is a legacy DRM node from the pre-KMS / early DRM model.
    // - Modern DRM userspace uses the primary node for display control and KMS, and
    //   uses the render node for rendering.
    // - There is no practical userspace dependency on a separate control node in the
    //   current Wayland/Mesa/virtio-gpu oriented design.
    //
    // let drm_minor = DrmMinor::new(index, device.clone(), DrmMinorType::Control);
    // char::register(drm_minor)?;
    let render_minor = if device.has_feature(DrmFeatures::RENDER) {
        let minor = DrmMinor::new(index, device.clone(), DrmMinorType::Render);
        char::register(minor.clone())?;
        Some(minor)
    } else {
        None
    };

    let primary_minor = DrmMinor::new(index, device.clone(), DrmMinorType::Primary);

    if let Err(error) = char::register(primary_minor) {
        if let Some(render_minor) = render_minor {
            let _ = char::unregister(render_minor.id());
        }
        return Err(error);
    }

    Ok(())
}
