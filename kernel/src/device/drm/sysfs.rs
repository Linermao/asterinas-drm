// SPDX-License-Identifier: MPL-2.0

use alloc::format;

use aster_drm::DrmDeviceBusInfo;
use aster_systree::{
    BranchNodeFields, Error as SysTreeError, NormalNodeFields, Result as SysTreeResult,
    SymlinkNodeFields, SysAttrSetBuilder, SysObj, SysPerms, SysStr, inherit_sys_branch_node,
    inherit_sys_leaf_node, inherit_sys_symlink_node,
};
use aster_util::printer::VmPrinter;
use inherit_methods_macro::inherit_methods;

use crate::{
    device::drm::{DRM_MAJOR_ID, DRM_RENDER_MINOR_BASE},
    prelude::*,
};

/// Registers a minimal DRM sysfs tree.
///
/// This is intentionally small and should be treated as scaffolding for a real
/// DRM device model. It exposes the paths Mesa currently probes when matching a
/// primary node to a render node:
///
/// - `/sys/dev/char/226:0/{dev,uevent}`
/// - `/sys/dev/char/226:0/device/{vendor,device,revision,uevent}`
/// - `/sys/dev/char/226:0/device/drm/{card0,renderD128}`
/// - `/sys/class/drm/{card0,renderD128}`
pub(super) fn register_device(
    index: u32,
    has_render_node: bool,
    bus_info: Option<DrmDeviceBusInfo>,
) -> Result<()> {
    let Some(DrmDeviceBusInfo::Pci {
        vendor_id,
        device_id,
    }) = bus_info
    else {
        return Ok(());
    };

    let bus_node = FolderNode::new("bus");
    bus_node.add_child(FolderNode::new("pci"))?;

    let dev_node = FolderNode::new("dev");
    let char_node = FolderNode::new("char");
    dev_node.add_child(char_node.clone())?;

    let primary_minor = index;
    let primary_name = format!("{}:{}", DRM_MAJOR_ID, primary_minor);
    let primary_char_node =
        DrmMinorBranchNode::new(primary_name, primary_minor, format!("dri/card{}", index));
    primary_char_node.add_child(build_device_node(
        index,
        has_render_node,
        vendor_id,
        device_id,
    )?)?;
    char_node.add_child(primary_char_node)?;

    if has_render_node {
        let render_minor = index + DRM_RENDER_MINOR_BASE;
        let render_name = format!("{}:{}", DRM_MAJOR_ID, render_minor);
        let render_char_node = DrmMinorBranchNode::new(
            render_name,
            render_minor,
            format!("dri/renderD{}", render_minor),
        );
        render_char_node.add_child(build_device_node(
            index,
            has_render_node,
            vendor_id,
            device_id,
        )?)?;
        char_node.add_child(render_char_node)?;
    }

    let class_node = FolderNode::new("class");
    let class_drm_node = FolderNode::new("drm");
    class_drm_node.add_child(build_class_minor_node(
        format!("card{}", index),
        primary_minor,
        format!("dri/card{}", index),
    )?)?;

    if has_render_node {
        let render_minor = index + DRM_RENDER_MINOR_BASE;
        class_drm_node.add_child(build_class_minor_node(
            format!("renderD{}", render_minor),
            render_minor,
            format!("dri/renderD{}", render_minor),
        )?)?;
    }

    class_node.add_child(class_drm_node)?;

    crate::fs::sysfs::systree_singleton()
        .root()
        .add_child(bus_node)?;

    crate::fs::sysfs::systree_singleton()
        .root()
        .add_child(dev_node)?;

    crate::fs::sysfs::systree_singleton()
        .root()
        .add_child(class_node)?;

    Ok(())
}

fn build_device_node(
    index: u32,
    has_render_node: bool,
    vendor_id: u16,
    device_id: u16,
) -> SysTreeResult<Arc<DeviceNode>> {
    let device_node = DeviceNode::new("device", index, vendor_id, device_id);
    let drm_node = FolderNode::new("drm");

    let primary_minor = index;
    drm_node.add_child(DrmMinorNode::new(
        format!("card{}", index),
        primary_minor,
        format!("dri/card{}", index),
    ))?;

    if has_render_node {
        let render_minor = index + DRM_RENDER_MINOR_BASE;
        drm_node.add_child(DrmMinorNode::new(
            format!("renderD{}", render_minor),
            render_minor,
            format!("dri/renderD{}", render_minor),
        ))?;
    }

    device_node.add_child(drm_node)?;
    device_node.add_child(SymlinkNode::new("subsystem", "/sys/bus/pci"))?;
    Ok(device_node)
}

fn build_class_minor_node(
    node_name: String,
    minor: u32,
    dev_name: String,
) -> SysTreeResult<Arc<DrmMinorBranchNode>> {
    let class_node = DrmMinorBranchNode::new(node_name, minor, dev_name);
    class_node.add_child(SymlinkNode::new("subsystem", "/sys/class/drm"))?;
    class_node.add_child(SymlinkNode::new(
        "device",
        format!("/sys/dev/char/{}:{}/device", DRM_MAJOR_ID, minor),
    ))?;
    Ok(class_node)
}

#[derive(Debug)]
struct FolderNode {
    fields: BranchNodeFields<dyn SysObj, Self>,
}

#[inherit_methods(from = "self.fields")]
impl FolderNode {
    fn new(name: impl Into<SysStr>) -> Arc<Self> {
        let name = name.into();
        Arc::new_cyclic(|weak_self| {
            let fields = BranchNodeFields::new(
                name,
                SysAttrSetBuilder::new().build().unwrap(),
                weak_self.clone(),
            );

            FolderNode { fields }
        })
    }

    fn add_child(&self, new_child: Arc<dyn SysObj>) -> SysTreeResult<()>;
}

inherit_sys_branch_node!(FolderNode, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }
});

#[derive(Debug)]
struct SymlinkNode {
    fields: SymlinkNodeFields<Self>,
}

impl SymlinkNode {
    fn new(name: impl Into<SysStr>, target_path: impl Into<String>) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| {
            let fields = SymlinkNodeFields::new(name.into(), target_path.into(), weak_self.clone());

            SymlinkNode { fields }
        })
    }
}

inherit_sys_symlink_node!(SymlinkNode, fields);

#[derive(Debug)]
struct DeviceNode {
    fields: BranchNodeFields<dyn SysObj, Self>,
    index: u32,
    vendor_id: u16,
    device_id: u16,
}

#[inherit_methods(from = "self.fields")]
impl DeviceNode {
    fn new(name: &'static str, index: u32, vendor_id: u16, device_id: u16) -> Arc<Self> {
        let mut builder = SysAttrSetBuilder::new();
        builder.add(SysStr::from("device"), SysPerms::DEFAULT_RO_ATTR_PERMS);
        builder.add(SysStr::from("revision"), SysPerms::DEFAULT_RO_ATTR_PERMS);
        builder.add(
            SysStr::from("subsystem_device"),
            SysPerms::DEFAULT_RO_ATTR_PERMS,
        );
        builder.add(
            SysStr::from("subsystem_vendor"),
            SysPerms::DEFAULT_RO_ATTR_PERMS,
        );
        builder.add(SysStr::from("uevent"), SysPerms::DEFAULT_RO_ATTR_PERMS);
        builder.add(SysStr::from("vendor"), SysPerms::DEFAULT_RO_ATTR_PERMS);
        let attrs = builder.build().unwrap();

        Arc::new_cyclic(|weak_self| {
            let fields = BranchNodeFields::new(SysStr::from(name), attrs, weak_self.clone());

            DeviceNode {
                fields,
                index,
                vendor_id,
                device_id,
            }
        })
    }

    fn add_child(&self, new_child: Arc<dyn SysObj>) -> SysTreeResult<()>;
}

inherit_sys_branch_node!(DeviceNode, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }

    fn read_attr_at(
        &self,
        name: &str,
        offset: usize,
        writer: &mut VmWriter,
    ) -> SysTreeResult<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);
        match name {
            "device" => writeln!(printer, "0x{:04x}", self.device_id)?,
            "revision" => writeln!(printer, "0x00")?,
            "subsystem_device" => writeln!(printer, "0x{:04x}", self.device_id)?,
            "subsystem_vendor" => writeln!(printer, "0x{:04x}", self.vendor_id)?,
            "uevent" => writeln!(printer, "PCI_SLOT_NAME=0000:00:{:02x}.0", self.index & 0xff)?,
            "vendor" => writeln!(printer, "0x{:04x}", self.vendor_id)?,
            _ => return Err(SysTreeError::NotFound),
        }
        Ok(printer.bytes_written())
    }

    fn write_attr(&self, _name: &str, _reader: &mut VmReader) -> SysTreeResult<usize> {
        Err(SysTreeError::PermissionDenied)
    }
});

#[derive(Debug)]
struct DrmMinorBranchNode {
    fields: BranchNodeFields<dyn SysObj, Self>,
    minor: u32,
    dev_name: String,
}

#[inherit_methods(from = "self.fields")]
impl DrmMinorBranchNode {
    fn new(name: impl Into<SysStr>, minor: u32, dev_name: String) -> Arc<Self> {
        let mut builder = SysAttrSetBuilder::new();
        builder.add(SysStr::from("dev"), SysPerms::DEFAULT_RO_ATTR_PERMS);
        builder.add(SysStr::from("uevent"), SysPerms::DEFAULT_RO_ATTR_PERMS);
        let attrs = builder.build().unwrap();

        Arc::new_cyclic(|weak_self| {
            let fields = BranchNodeFields::new(name.into(), attrs, weak_self.clone());

            DrmMinorBranchNode {
                fields,
                minor,
                dev_name,
            }
        })
    }

    fn add_child(&self, new_child: Arc<dyn SysObj>) -> SysTreeResult<()>;
}

inherit_sys_branch_node!(DrmMinorBranchNode, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }

    fn read_attr_at(
        &self,
        name: &str,
        offset: usize,
        writer: &mut VmWriter,
    ) -> SysTreeResult<usize> {
        write_drm_minor_attr(name, self.minor, &self.dev_name, offset, writer)
    }

    fn write_attr(&self, _name: &str, _reader: &mut VmReader) -> SysTreeResult<usize> {
        Err(SysTreeError::PermissionDenied)
    }
});

#[derive(Debug)]
struct DrmMinorNode {
    fields: NormalNodeFields<Self>,
    minor: u32,
    dev_name: String,
}

impl DrmMinorNode {
    fn new(name: String, minor: u32, dev_name: String) -> Arc<Self> {
        let mut builder = SysAttrSetBuilder::new();
        builder.add(SysStr::from("dev"), SysPerms::DEFAULT_RO_ATTR_PERMS);
        builder.add(SysStr::from("uevent"), SysPerms::DEFAULT_RO_ATTR_PERMS);
        let attrs = builder.build().unwrap();

        Arc::new_cyclic(|weak_self| {
            let fields = NormalNodeFields::new(SysStr::from(name), attrs, weak_self.clone());

            DrmMinorNode {
                fields,
                minor,
                dev_name,
            }
        })
    }
}

inherit_sys_leaf_node!(DrmMinorNode, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }

    fn read_attr_at(
        &self,
        name: &str,
        offset: usize,
        writer: &mut VmWriter,
    ) -> SysTreeResult<usize> {
        write_drm_minor_attr(name, self.minor, &self.dev_name, offset, writer)
    }

    fn write_attr(&self, _name: &str, _reader: &mut VmReader) -> SysTreeResult<usize> {
        Err(SysTreeError::PermissionDenied)
    }
});

fn write_drm_minor_attr(
    name: &str,
    minor: u32,
    dev_name: &str,
    offset: usize,
    writer: &mut VmWriter,
) -> SysTreeResult<usize> {
    let mut printer = VmPrinter::new_skip(writer, offset);

    match name {
        "dev" => {
            writeln!(printer, "{}:{}", DRM_MAJOR_ID, minor)?;
        }
        "uevent" => {
            writeln!(printer, "MAJOR={}", DRM_MAJOR_ID)?;
            writeln!(printer, "MINOR={}", minor)?;
            writeln!(printer, "DEVNAME={}", dev_name)?;
            writeln!(printer, "DEVTYPE=drm_minor")?;
        }
        _ => return Err(SysTreeError::NotFound),
    }

    Ok(printer.bytes_written())
}
