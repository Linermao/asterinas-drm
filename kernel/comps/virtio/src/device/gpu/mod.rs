use core::mem::offset_of;
use bitflags::bitflags;
use crate::transport::{ConfigManager, VirtioTransport};

pub(crate) mod device;
pub(crate) mod gem;
pub(crate) mod output;

#[repr(u16)]
pub(crate) enum VirtioGpuQueue {
    QueueControl = 0,
    QueueCursor = 1,
}

#[repr(u32)]
pub(crate) enum VirtioGpuRespOk {
    Nodata = 0x1100,
    DisplayInfo = 0x1101,
    CapsetInfo = 0x1102,
    Edid = 0x1104,
}

#[repr(u32)]
pub(crate) enum VirtioGpuCmd {
    GetDisplayInfo = 0x0100,
    ResourceCreate2D = 0x0101,
    ResourceUnref = 0x0102,
    ResourceFlush = 0x0104,
    TransferToHost2D = 0x0105,
    SetScanout = 0x0103,
    ResourceAttachBacking = 0x0106,
    ResourceDetachBacking = 0x0107,
    GetCapsetInfo = 0x0108,
    GetEdid = 0x010a,
}

bitflags! {
    /// VirtIO GPU features defined by the specification.
    pub(crate) struct GpuFeatures: u64 {
        const VIRGL       = 1 << 0;
        const EDID        = 1 << 1;
        const RESOURCE_UUID = 1 << 2;
        const RESOURCE_BLOB = 1 << 3;
        const CONTEXT_INIT  = 1 << 4;
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct VirtioGpuConfig {
    pub events_read: u32,
    pub events_clear: u32,
    pub num_scanouts: u32,
    pub num_capsets: u32,
}

impl VirtioGpuConfig {
    pub(super) fn new_manager(transport: &dyn VirtioTransport) -> ConfigManager<Self> {
        let safe_ptr = transport.device_config_mem().map(|mem| {
            // The config starts from offset 0.
            aster_util::safe_ptr::SafePtr::new(mem, 0)
        });
        let bar_space = transport.device_config_bar();
        ConfigManager::new(safe_ptr, bar_space)
    }
}

impl ConfigManager<VirtioGpuConfig> {
    pub(super) fn read_config(&self) -> VirtioGpuConfig {
        VirtioGpuConfig {
            events_read: self
                .read_once::<u32>(offset_of!(VirtioGpuConfig, events_read))
                .unwrap(),
            events_clear: self
                .read_once::<u32>(offset_of!(VirtioGpuConfig, events_clear))
                .unwrap(),
            num_scanouts: self
                .read_once::<u32>(offset_of!(VirtioGpuConfig, num_scanouts))
                .unwrap(),
            num_capsets: self
                .read_once::<u32>(offset_of!(VirtioGpuConfig, num_capsets))
                .unwrap(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuCtrlHdr {
    pub type_: u32,
    pub flags: u32,
    pub fence_id: u64,
    pub ctx_id: u32,
    pub ring_idx: u8,
    pub padding: [u8; 3],
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuDisplayOne {
    pub rect: VirtioGpuRect,
    pub enabled: u32,
    pub flags: u32,
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuRespDisplayInfo {
    pub hdr: VirtioGpuCtrlHdr,
    pub pmodes: [VirtioGpuDisplayOne; 16],
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuGetCapsetInfo {
    pub hdr: VirtioGpuCtrlHdr,
    pub capset_index: u32,
    pub padding: u32,
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuRespCapsetInfo {
    pub hdr: VirtioGpuCtrlHdr,
    pub capset_id: u32,
    pub capset_max_version: u32,
    pub capset_max_size: u32,
    pub padding: u32,
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuGetEdid {
    pub hdr: VirtioGpuCtrlHdr,
    pub scanout: u32,
    pub padding: u32,
}

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct VirtioGpuRespEdid {
    pub hdr: VirtioGpuCtrlHdr,
    pub size: u32,
    pub padding: u32,
    pub edid: [u8; 1024],
}

impl Default for VirtioGpuRespEdid {
    fn default() -> Self {
        Self {
            hdr: VirtioGpuCtrlHdr::default(),
            size: 0,
            padding: 0,
            edid: [0; 1024],
        }
    }
}

#[repr(u32)]
#[derive(Debug, Clone, Copy)]
pub enum VirtioGpuFormat {
    B8G8R8X8Unorm = 2,
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuResourceCreate2d {
    pub hdr: VirtioGpuCtrlHdr,
    pub resource_id: u32,
    pub format: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuResourceUnref {
    pub hdr: VirtioGpuCtrlHdr,
    pub resource_id: u32,
    pub padding: u32,
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuMemEntry {
    pub addr: u64,
    pub length: u32,
    pub padding: u32,
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuResourceAttachBacking {
    pub hdr: VirtioGpuCtrlHdr,
    pub resource_id: u32,
    pub nr_entries: u32,
    pub entries: [VirtioGpuMemEntry; 1],
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuResourceDetachBacking {
    pub hdr: VirtioGpuCtrlHdr,
    pub resource_id: u32,
    pub padding: u32,
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuResourceFlush {
    pub hdr: VirtioGpuCtrlHdr,
    pub rect: VirtioGpuRect,
    pub resource_id: u32,
    pub _padding: u32,
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuTransferToHost2d {
    pub hdr: VirtioGpuCtrlHdr,
    pub rect: VirtioGpuRect,
    pub offset: u64,
    pub resource_id: u32,
    pub _padding: u32,
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuSetScanout {
    pub hdr: VirtioGpuCtrlHdr,
    pub rect: VirtioGpuRect,
    pub scanout_id: u32,
    pub resource_id: u32,
}
