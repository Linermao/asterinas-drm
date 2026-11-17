// SPDX-License-Identifier: MPL-2.0

//! DRM device registration in the shared device model.
//!
//! Each DRM device has a logical parent at `/sys/devices/drmN`, with its minors
//! under `drm/cardN` and, when supported, a corresponding render node. The device model creates
//! the `/sys/class/drm` and `/sys/dev/char` links, the `dev` attribute, the
//! `device` and `subsystem` links, and the corresponding `/dev/dri` nodes.
//! A read-only `uevent` attribute supplies the device name used by libdrm.

use alloc::{format, sync::Arc, vec::Vec};
use core::fmt::Write;

use aster_core::{device::Device, prelude::*};
use aster_device::{
    class::{self, Class, ClassDevice, ClassHandle},
    common::{AnyDevice, Attr, BareDevice, DevNode, DevNum},
};
use ostd::sync::Mutex;

use crate::minor::DrmMinor;

/// Publishes the parent and minors of a DRM device.
///
/// The minors must already be in the character-device registry so that opening
/// a newly published `/dev/dri` node can reach its DRM file operations.
pub(super) fn register(primary: Arc<DrmMinor>, render: Option<Arc<DrmMinor>>) -> Result<()> {
    let class = drm_class()?;
    // Hardware drivers do not yet supply device-model parents. Keep each DRM
    // device's minors together without claiming a PCI or platform attachment.
    let parent = BareDevice::new_root(format!("drm{}", primary.id().minor().get()));
    aster_device::add_device(&parent)?;

    let mut minors = Vec::new();
    for minor in core::iter::once(primary).chain(render) {
        let devnum = DevNum::char(minor.id());
        let node = ClassDevice::builder(&class, minor.name(), minor)
            .parent(parent.clone())
            .devnum(devnum)
            .build();
        if let Err(error) = aster_device::add_device(&node) {
            for minor in minors.iter().rev() {
                let _ = aster_device::remove_device(minor);
            }
            let _ = aster_device::remove_device(&parent);
            return Err(error.into());
        }
        minors.push(node);
    }

    // Registered class members and their parent are retained by the device model.
    Ok(())
}

fn drm_class() -> Result<Arc<ClassHandle<DrmClass>>> {
    static DRM_CLASS: Mutex<Option<Arc<ClassHandle<DrmClass>>>> = Mutex::new(None);

    let mut class = DRM_CLASS.lock();
    if let Some(class) = class.as_ref() {
        return Ok(class.clone());
    }

    let registered = class::register(DrmClass)?;
    *class = Some(registered.clone());
    Ok(registered)
}

struct DrmClass;

impl Class for DrmClass {
    const NAME: &'static str = "drm";
    type Device = Arc<DrmMinor>;

    fn devnode(&self, dev: &ClassDevice<Self>) -> Option<DevNode> {
        Some(DevNode {
            path: Some(format!("dri/{}", dev.payload().name()).into()),
            mode: None,
        })
    }

    fn dev_attrs(&self) -> &'static [Attr<ClassDevice<Self>>] {
        &DRM_DEVICE_ATTRS
    }
}

static DRM_DEVICE_ATTRS: [Attr<ClassDevice<DrmClass>>; 1] = [Attr::ro("uevent", show_uevent)];

fn show_uevent(
    dev: &ClassDevice<DrmClass>,
    writer: &mut dyn Write,
) -> aster_device::common::Result<()> {
    // Reference: <https://elixir.bootlin.com/linux/v6.18/source/drivers/base/core.c>.
    // These are the device-number properties produced by Linux's dev_uevent().
    let id = dev
        .devnum()
        .ok_or(aster_device::common::Error::NoDevNum)?
        .id();
    writeln!(writer, "MAJOR={}", id.major().get())?;
    writeln!(writer, "MINOR={}", id.minor().get())?;
    writeln!(writer, "DEVNAME=dri/{}", dev.payload().name())?;
    Ok(())
}
