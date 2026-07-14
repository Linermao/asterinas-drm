// SPDX-License-Identifier: MPL-2.0

use aster_drm::DrmSgEntry;
use int_to_c_enum::TryFromInt;

pub const VIRTIO_GPU_MAX_EDID_SIZE: usize = 1024;
pub const VIRTIO_GPU_MAX_SCANOUTS: usize = 16;
pub const VIRTIO_GPU_MAX_DEBUG_NAME: usize = 64;

bitflags::bitflags! {
    pub struct VirtioGpuFlags: u32 {
        const FENCE = 1 << 0;
        const RING_IDX = 1 << 1;
    }
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub enum VirtioGpuCtrlType {
    Unknown = 0x0000,
    GetDisplayInfo = 0x0100,
    ResourceCreate2d = 0x0101,
    ResourceUnref = 0x0102,
    SetScanout = 0x0103,
    ResourceFlush = 0x0104,
    TransferToHost2d = 0x0105,
    ResourceAttachBacking = 0x0106,
    ResourceDetachBacking = 0x0107,
    GetCapsetInfo = 0x0108,
    GetCapset = 0x0109,
    GetEdid = 0x010a,
    ResourceAssignUuid = 0x010b,
    ResourceCreateBlob = 0x010c,
    CtxCreate = 0x0200,
    CtxDestroy = 0x0201,
    CtxAttachResource = 0x0202,
    CtxDetachResource = 0x0203,
    ResourceCreate3d = 0x0204,
    TransferToHost3d = 0x0205,
    TransferFromHost3d = 0x0206,
    CmdSubmit3d = 0x0207,
    ResourceMapBlob = 0x0208,
    ResourceUnmapBlob = 0x0209,
    RespOkNodata = 0x1100,
    RespOkDisplayInfo = 0x1101,
    RespOkCapsetInfo = 0x1102,
    RespOkCapset = 0x1103,
    RespOkEdid = 0x1104,
    RespOkMapInfo = 0x1106,
    RespErrUnspec = 0x1200,
    RespErrOutOfMemory = 0x1201,
    RespErrInvalidScanoutId = 0x1202,
    RespErrInvalidResourceId = 0x1203,
    RespErrInvalidContextId = 0x1204,
    RespErrInvalidParameter = 0x1205,
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VirtioGpuFormat {
    B8G8R8A8Unorm = 1,
    B8G8R8X8Unorm = 2,
    A8R8G8B8Unorm = 3,
    X8R8G8B8Unorm = 4,
    R8G8B8A8Unorm = 67,
    X8B8G8R8Unorm = 68,
    A8B8G8R8Unorm = 121,
    R8G8B8X8Unorm = 134,
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

impl VirtioGpuCtrlHdr {
    pub fn new(type_: VirtioGpuCtrlType) -> Self {
        Self {
            type_: type_ as u32,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct VirtioGpuCtxCreate {
    pub hdr: VirtioGpuCtrlHdr,
    pub nlen: u32,
    pub context_init: u32,
    pub debug_name: [u8; VIRTIO_GPU_MAX_DEBUG_NAME],
}

impl VirtioGpuCtxCreate {
    pub fn new(
        context_id: u32,
        context_init: u32,
        nlen: u32,
        debug_name: [u8; VIRTIO_GPU_MAX_DEBUG_NAME],
    ) -> Self {
        let mut hdr = VirtioGpuCtrlHdr::new(VirtioGpuCtrlType::CtxCreate);
        hdr.ctx_id = context_id;

        Self {
            hdr,
            nlen,
            context_init,
            debug_name,
        }
    }
}

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct VirtioGpuCtxDestroy {
    pub hdr: VirtioGpuCtrlHdr,
}

impl VirtioGpuCtxDestroy {
    pub fn new(context_id: u32) -> Self {
        let mut hdr = VirtioGpuCtrlHdr::new(VirtioGpuCtrlType::CtxDestroy);
        hdr.ctx_id = context_id;

        Self {
            hdr
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuCtxResource {
    pub hdr: VirtioGpuCtrlHdr,
    pub resource_id: u32,
    pub padding: u32,
}

impl VirtioGpuCtxResource {
    pub fn new(ctx_id: u32, resource_id: u32) -> Self {
        let mut hdr = VirtioGpuCtrlHdr::new(VirtioGpuCtrlType::CtxAttachResource);
        hdr.ctx_id = ctx_id;

        Self {
            hdr,
            resource_id,
            padding: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuCmdSubmit {
    pub hdr: VirtioGpuCtrlHdr,
    pub size: u32,
    pub padding: u32,
}

impl VirtioGpuCmdSubmit {
    pub fn new(size: u32, context_id: u32, ring_idx: Option<u8>) -> Self {
        let mut header = VirtioGpuCtrlHdr::new(VirtioGpuCtrlType::CmdSubmit3d);
        header.ctx_id = context_id;

        if let Some(ring_idx) = ring_idx {
            header.flags |= VirtioGpuFlags::RING_IDX.bits();
            header.ring_idx = ring_idx;
        }

        Self {
            hdr: header,
            size,
            padding: 0,
        }
    }
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

impl VirtioGpuResourceCreate2d {
    pub fn new(resource_id: u32, format: VirtioGpuFormat, width: u32, height: u32) -> Self {
        Self {
            hdr: VirtioGpuCtrlHdr::new(VirtioGpuCtrlType::ResourceCreate2d),
            resource_id,
            format: format as u32,
            width,
            height,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuResourceCreate3d {
    pub hdr: VirtioGpuCtrlHdr,
    pub resource_id: u32,
    pub target: u32,
    pub format: u32,
    pub bind: u32,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub array_size: u32,
    pub last_level: u32,
    pub nr_samples: u32,
    pub flags: u32,
    pub padding: u32,
}

impl VirtioGpuResourceCreate3d {
    pub fn new(
        resource_id: u32,
        target: u32,
        format: u32,
        bind: u32,
        width: u32,
        height: u32,
        depth: u32,
        array_size: u32,
        last_level: u32,
        nr_samples: u32,
        flags: u32,
    ) -> Self {
        Self {
            hdr: VirtioGpuCtrlHdr::new(VirtioGpuCtrlType::ResourceCreate3d),
            resource_id,
            target,
            format,
            bind,
            width,
            height,
            depth,
            array_size,
            last_level,
            nr_samples,
            flags,
            padding: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuResourceAttachBacking {
    pub hdr: VirtioGpuCtrlHdr,
    pub resource_id: u32,
    pub nr_entries: u32,
}

impl VirtioGpuResourceAttachBacking {
    pub fn new(resource_id: u32, nr_entries: u32) -> Self {
        Self {
            hdr: VirtioGpuCtrlHdr::new(VirtioGpuCtrlType::ResourceAttachBacking),
            resource_id,
            nr_entries,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuResourceDetachBacking {
    pub hdr: VirtioGpuCtrlHdr,
    pub resource_id: u32,
    pub padding: u32,
}

impl VirtioGpuResourceDetachBacking {
    pub fn new(resource_id: u32) -> Self {
        Self {
            hdr: VirtioGpuCtrlHdr::new(VirtioGpuCtrlType::ResourceDetachBacking),
            resource_id,
            padding: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuMemEntry {
    pub addr: u64,
    pub length: u32,
    pub padding: u32,
}

impl From<DrmSgEntry> for VirtioGpuMemEntry {
    fn from(entry: DrmSgEntry) -> Self {
        Self {
            addr: entry.addr(),
            length: entry.length(),
            padding: 0,
        }
    }
}

impl VirtioGpuMemEntry {
    pub fn new(addr: u64, length: u32) -> Self {
        Self {
            addr,
            length,
            padding: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl VirtioGpuRect {
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuSetScanout {
    pub hdr: VirtioGpuCtrlHdr,
    pub rect: VirtioGpuRect,
    pub scanout_id: u32,
    pub resource_id: u32,
}

impl VirtioGpuSetScanout {
    pub fn new(scanout_id: u32, resource_id: u32, rect: VirtioGpuRect) -> Self {
        Self {
            hdr: VirtioGpuCtrlHdr::new(VirtioGpuCtrlType::SetScanout),
            rect,
            scanout_id,
            resource_id,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuResourceFlush {
    pub hdr: VirtioGpuCtrlHdr,
    pub rect: VirtioGpuRect,
    pub resource_id: u32,
    pub padding: u32,
}

impl VirtioGpuResourceFlush {
    pub fn new(resource_id: u32, rect: VirtioGpuRect) -> Self {
        Self {
            hdr: VirtioGpuCtrlHdr::new(VirtioGpuCtrlType::ResourceFlush),
            rect,
            resource_id,
            padding: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuTransferToHost2d {
    pub hdr: VirtioGpuCtrlHdr,
    pub rect: VirtioGpuRect,
    pub offset: u64,
    pub resource_id: u32,
    pub padding: u32,
}

impl VirtioGpuTransferToHost2d {
    pub fn new(resource_id: u32, rect: VirtioGpuRect, offset: u64) -> Self {
        Self {
            hdr: VirtioGpuCtrlHdr::new(VirtioGpuCtrlType::TransferToHost2d),
            rect,
            offset,
            resource_id,
            padding: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuBox {
    pub x: u32,
    pub y: u32,
    pub z: u32,
    pub w: u32,
    pub h: u32,
    pub d: u32,
}

impl VirtioGpuBox {
    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0 || self.d == 0
    }
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuTransferHost3d {
    pub hdr: VirtioGpuCtrlHdr,
    pub box_: VirtioGpuBox,
    pub offset: u64,
    pub resource_id: u32,
    pub level: u32,
    pub stride: u32,
    pub layer_stride: u32,
}

impl VirtioGpuTransferHost3d {
    pub fn new(
        type_: VirtioGpuCtrlType,
        context_id: u32,
        resource_id: u32,
        box_: VirtioGpuBox,
        offset: u64,
        level: u32,
        stride: u32,
        layer_stride: u32,
    ) -> Self {
        let mut hdr = VirtioGpuCtrlHdr::new(type_);
        hdr.ctx_id = context_id;

        Self {
            hdr,
            box_,
            offset,
            resource_id,
            level,
            stride,
            layer_stride,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuResourceCreateBlob {
    pub hdr: VirtioGpuCtrlHdr,
    pub resource_id: u32,
    pub blob_mem: u32,
    pub blob_flags: u32,
    pub nr_entries: u32,
    pub blob_id: u64,
    pub size: u64,
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuResourceMapBlob {
    pub hdr: VirtioGpuCtrlHdr,
    pub resource_id: u32,
    pub padding: u32,
    pub offset: u64,
}

impl VirtioGpuResourceMapBlob {
    pub fn new(resource_id: u32, offset: u64) -> Self {
        Self {
            hdr: VirtioGpuCtrlHdr::new(VirtioGpuCtrlType::ResourceMapBlob),
            resource_id,
            padding: 0,
            offset,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuRespMapInfo {
    pub hdr: VirtioGpuCtrlHdr,
    pub map_info: u32,
    pub padding: u32,
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuResourceUnmapBlob {
    pub hdr: VirtioGpuCtrlHdr,
    pub resource_id: u32,
    pub padding: u32,
}

impl VirtioGpuResourceUnmapBlob {
    pub fn new(resource_id: u32) -> Self {
        Self {
            hdr: VirtioGpuCtrlHdr::new(VirtioGpuCtrlType::ResourceUnmapBlob),
            resource_id,
            padding: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuResourceUnref {
    pub hdr: VirtioGpuCtrlHdr,
    pub resource_id: u32,
    pub padding: u32,
}

impl VirtioGpuResourceUnref {
    pub fn new(resource_id: u32) -> Self {
        Self {
            hdr: VirtioGpuCtrlHdr::new(VirtioGpuCtrlType::ResourceUnref),
            resource_id,
            padding: 0,
        }
    }
}

impl VirtioGpuResourceCreateBlob {
    pub fn new(
        context_id: u32,
        resource_id: u32,
        blob_mem: u32,
        blob_flags: u32,
        nr_entries: u32,
        blob_id: u64,
        size: u64,
    ) -> Self {
        let mut hdr = VirtioGpuCtrlHdr::new(VirtioGpuCtrlType::ResourceCreateBlob);
        hdr.ctx_id = context_id;

        Self {
            hdr,
            resource_id,
            blob_mem,
            blob_flags,
            nr_entries,
            blob_id,
            size,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuDisplayOne {
    pub rect: VirtioGpuRect,
    pub enabled: u32,
    pub flags: u32,
}

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct VirtioGpuRespDisplayInfo {
    pub hdr: VirtioGpuCtrlHdr,
    pub pmodes: [VirtioGpuDisplayOne; VIRTIO_GPU_MAX_SCANOUTS],
}

#[derive(Debug, Clone, Copy)]
pub struct VirtioGpuDisplayInfo {
    rect: VirtioGpuRect,
    enabled: bool,
    flags: u32,
}

impl VirtioGpuDisplayInfo {
    pub fn new(display: VirtioGpuDisplayOne) -> Self {
        Self {
            rect: display.rect,
            enabled: display.enabled != 0,
            flags: display.flags,
        }
    }

    pub fn rect(&self) -> VirtioGpuRect {
        self.rect
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn flags(&self) -> u32 {
        self.flags
    }
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuGetEdid {
    pub hdr: VirtioGpuCtrlHdr,
    pub scanout: u32,
    pub padding: u32,
}

impl VirtioGpuGetEdid {
    pub fn new(scanout: u32) -> Self {
        Self {
            hdr: VirtioGpuCtrlHdr::new(VirtioGpuCtrlType::GetEdid),
            scanout,
            padding: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct VirtioGpuRespEdid {
    pub hdr: VirtioGpuCtrlHdr,
    pub size: u32,
    pub padding: u32,
    pub edid: [u8; VIRTIO_GPU_MAX_EDID_SIZE],
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuGetCapsetInfo {
    pub hdr: VirtioGpuCtrlHdr,
    pub capset_index: u32,
    pub padding: u32,
}

impl VirtioGpuGetCapsetInfo {
    pub fn new(capset_index: u32) -> Self {
        Self {
            hdr: VirtioGpuCtrlHdr::new(VirtioGpuCtrlType::GetCapsetInfo),
            capset_index,
            padding: 0,
        }
    }
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

#[derive(Debug, Clone, Copy)]
pub struct VirtioGpuCapsetInfo {
    id: u32,
    max_version: u32,
    max_size: u32,
}

impl VirtioGpuCapsetInfo {
    pub fn new(info: VirtioGpuRespCapsetInfo) -> Self {
        Self {
            id: info.capset_id,
            max_version: info.capset_max_version,
            max_size: info.capset_max_size,
        }
    }

    pub fn id(&self) -> u32 {
        self.id
    }

    pub fn max_version(&self) -> u32 {
        self.max_version
    }

    pub fn max_size(&self) -> u32 {
        self.max_size
    }
}

#[derive(Debug, Clone, Copy, Default, Pod)]
#[repr(C)]
pub struct VirtioGpuGetCapset {
    pub hdr: VirtioGpuCtrlHdr,
    pub capset_id: u32,
    pub capset_version: u32,
}

impl VirtioGpuGetCapset {
    pub fn new(capset_id: u32, capset_version: u32) -> Self {
        Self {
            hdr: VirtioGpuCtrlHdr::new(VirtioGpuCtrlType::GetCapset),
            capset_id,
            capset_version,
        }
    }
}
