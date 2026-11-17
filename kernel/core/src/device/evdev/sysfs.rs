// SPDX-License-Identifier: MPL-2.0

//! Input class registration for evdev discovery.
//!
//! Each input device has an `inputN` parent and an `eventN` child. The device
//! model publishes `/sys/class/input`, `/sys/dev/char` links and the `/dev/input`
//! node. Capability bitmaps in the parent's `uevent` let libudev-zero classify
//! devices without a udev daemon.

use alloc::format;
use core::fmt::Write;

use aster_device::{
    class::{self, Class, ClassDevice, ClassHandle},
    common::{AnyDevice, Attr, DevNode, DevNum, Error, Result},
};
use device_id::MinorId;
use spin::Once;

use super::EvdevDevice;
use crate::{device::Device, prelude::*};

static INPUT_CLASS: Once<Arc<ClassHandle<InputClass>>> = Once::new();

pub(super) fn init() {
    INPUT_CLASS.call_once(|| class::register(InputClass).unwrap());
}

pub(super) fn register(evdev: Arc<EvdevDevice>) -> Result<()> {
    let class = INPUT_CLASS.get().unwrap();
    let minor = evdev.id().minor().get();
    // InputDevice does not yet supply a device-model hardware parent. Keep a
    // logical input parent without inventing an i8042 or virtio attachment.
    let input = ClassDevice::builder(class, format!("input{minor}"), evdev.clone())
        .attrs(&INPUT_ATTRS)
        .build();
    aster_device::add_device(&input)?;

    let event = ClassDevice::builder(class, format!("event{minor}"), evdev.clone())
        .parent(input.clone())
        .devnum(DevNum::char(evdev.id()))
        .attrs(&EVENT_ATTRS)
        .build();
    if let Err(error) = aster_device::add_device(&event) {
        let _ = aster_device::remove_device(&input);
        return Err(error);
    }
    Ok(())
}

pub(super) fn unregister(minor: MinorId) -> Result<()> {
    let class = INPUT_CLASS.get().unwrap();
    let minor = minor.get();
    let event = class
        .find_device(&format!("event{minor}"))
        .ok_or(Error::NotFound)?;
    aster_device::remove_device(&event)?;
    let input = class
        .find_device(&format!("input{minor}"))
        .ok_or(Error::NotFound)?;
    aster_device::remove_device(&input)
}

struct InputClass;

impl Class for InputClass {
    const NAME: &'static str = "input";
    type Device = Arc<EvdevDevice>;

    fn devnode(&self, dev: &ClassDevice<Self>) -> Option<DevNode> {
        dev.devnum().map(|devnum| DevNode {
            path: Some(format!("input/event{}", devnum.id().minor().get()).into()),
            mode: None,
        })
    }

    fn dev_attrs(&self) -> &'static [Attr<ClassDevice<Self>>] {
        &[]
    }
}

type InputNode = ClassDevice<InputClass>;

static INPUT_ATTRS: [Attr<InputNode>; 5] = [
    Attr::ro("name", show_name),
    Attr::ro("phys", show_phys),
    Attr::ro("uniq", show_uniq),
    Attr::ro("properties", show_properties),
    Attr::ro("uevent", show_input_uevent),
];

static EVENT_ATTRS: [Attr<InputNode>; 1] = [Attr::ro("uevent", show_event_uevent)];

fn show_name(dev: &InputNode, writer: &mut dyn Write) -> Result<()> {
    writeln!(writer, "{}", dev.payload().device.name())?;
    Ok(())
}

fn show_phys(dev: &InputNode, writer: &mut dyn Write) -> Result<()> {
    writeln!(writer, "{}", dev.payload().device.phys())?;
    Ok(())
}

fn show_uniq(dev: &InputNode, writer: &mut dyn Write) -> Result<()> {
    writeln!(writer, "{}", dev.payload().device.uniq())?;
    Ok(())
}

fn show_properties(_dev: &InputNode, writer: &mut dyn Write) -> Result<()> {
    // InputDevice currently has no INPUT_PROP_* capabilities.
    writeln!(writer, "0")?;
    Ok(())
}

fn show_input_uevent(dev: &InputNode, writer: &mut dyn Write) -> Result<()> {
    // Linux input_dev_uevent() exports capability words most significant first.
    // libudev-zero reads EV/KEY/REL/PROP here to produce ID_INPUT_* properties:
    // <https://github.com/illiliti/libudev-zero/blob/1.0.4/udev_device.c>.
    let device = &dev.payload().device;
    let id = device.id();
    let capability = device.capability();
    writeln!(
        writer,
        "PRODUCT={:x}/{:x}/{:x}/{:x}",
        id.bustype(),
        id.vendor(),
        id.product(),
        id.version()
    )?;
    writeln!(writer, "NAME=\"{}\"", device.name())?;
    writeln!(writer, "PHYS=\"{}\"", device.phys())?;
    writeln!(writer, "UNIQ=\"{}\"", device.uniq())?;
    writeln!(writer, "PROP=0")?;
    write!(writer, "EV=")?;
    show_bitmap(&capability.event_types_bits().to_le_bytes(), writer)?;
    write!(writer, "KEY=")?;
    show_bitmap(capability.supported_keys_bitmap(), writer)?;
    write!(writer, "REL=")?;
    show_bitmap(capability.supported_relative_axes_bitmap(), writer)?;
    Ok(())
}

fn show_event_uevent(dev: &InputNode, writer: &mut dyn Write) -> Result<()> {
    let id = dev.payload().id();
    writeln!(writer, "MAJOR={}", id.major().get())?;
    writeln!(writer, "MINOR={}", id.minor().get())?;
    writeln!(writer, "DEVNAME=input/event{}", id.minor().get())?;
    Ok(())
}

/// Writes Linux's native-word hexadecimal bitmap format, including its newline.
fn show_bitmap(bitmap: &[u8], writer: &mut dyn Write) -> Result<()> {
    // InputCapability stores bit n in byte n / 8. Sysfs groups these bits into
    // unsigned long words, prints high words first, and omits leading zero words.
    let mut started = false;
    for bytes in bitmap.chunks(size_of::<usize>()).rev() {
        let mut word = 0usize;
        for (index, byte) in bytes.iter().enumerate() {
            word |= (*byte as usize) << (index * 8);
        }
        if !started && word == 0 {
            continue;
        }
        if started {
            write!(writer, " ")?;
        }
        write!(writer, "{word:x}")?;
        started = true;
    }
    if !started {
        write!(writer, "0")?;
    }
    writeln!(writer)?;
    Ok(())
}
