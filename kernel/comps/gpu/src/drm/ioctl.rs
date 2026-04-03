use int_to_c_enum::TryFromInt;

use crate::drm::{
    drm_modes::{DrmFormat, DrmModeModeInfo},
    objects::property::DRM_PROP_NAME_LEN,
};

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmVersion {
    pub version_major: i32,
    pub version_minor: i32,
    pub version_patchlevel: i32,
    _padding: u32,

    pub name_len: usize,
    pub name: u64,
    pub date_len: usize,
    pub date: u64,
    pub desc_len: usize,
    pub desc: u64,
}

impl DrmVersion {
    pub fn is_first_call(&self) -> bool {
        return self.name == 0 && self.date == 0 && self.desc == 0;
    }
}

#[repr(u64)]
#[derive(Debug, TryFromInt)]
pub enum DrmGetCapabilities {
    DumbBuffer = 0x1,
    VblankHighCrtc = 0x2,
    DumbPreferredDepth = 0x3,
    DumbPreferShadow = 0x4,
    Prime = 0x5,
    TimestampMonotonic = 0x6,
    AsyncPageFlip = 0x7,
    CursorWidth = 0x8,
    CursorHeight = 0x9,
    Addfb2Modifiers = 0x10,
    PageFlipTarget = 0x11,
    CrtcInVblankEvent = 0x12,
    SyncObj = 0x13,
    SyncObjTimeline = 0x14,
    AtomicAsyncPageFlip = 0x15,
}

bitflags::bitflags! {
    pub struct DrmPrimeValue: u64 {
        const IMPORT = 0x1;
        const EXPORT = 0x2;
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmGetCap {
    pub capability: u64,
    pub value: u64,
}

#[repr(u64)]
#[derive(Debug, TryFromInt)]
pub enum DrmSetCapabilities {
    Stereo3D = 0x1,
    UniversalPlane = 0x2,
    Atomic = 0x3,
    AspectRatio = 0x4,
    WritebackConnectors = 0x5,
    CursorPlaneHostport = 0x6,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmSetClientCap {
    pub capability: u64,
    pub value: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmModeGetResources {
    pub fb_id_ptr: u64,
    pub crtc_id_ptr: u64,
    pub connector_id_ptr: u64,
    pub encoder_id_ptr: u64,

    pub count_fbs: u32,
    pub count_crtcs: u32,
    pub count_connectors: u32,
    pub count_encoders: u32,

    pub min_width: u32,
    pub max_width: u32,
    pub min_height: u32,
    pub max_height: u32,
}

impl DrmModeGetResources {
    pub fn is_first_call(&self) -> bool {
        return self.fb_id_ptr == 0
            && self.crtc_id_ptr == 0
            && self.connector_id_ptr == 0
            && self.encoder_id_ptr == 0;
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmModeCrtc {
    pub set_connectors_ptr: u64,
    pub count_connectors: u32,

    pub crtc_id: u32,
    pub fb_id: u32,

    pub x: u32,
    pub y: u32,

    pub gamma_size: u32,
    pub mode_valid: u32,
    pub mode: DrmModeModeInfo,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmModeCursor {
    pub flags: u32,
    pub crtc_id: u32,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub handle: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmModeCursor2 {
    pub flags: u32,
    pub crtc_id: u32,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub handle: u32,
    pub hot_x: i32,
    pub hot_y: i32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmModeAtomic {
    pub flags: u32,
    pub count_objs: u32,
    pub objs_ptr: u64,
    pub count_props_ptr: u64,
    pub props_ptr: u64,
    pub prop_values_ptr: u64,
    pub reserved: u64,
    pub user_data: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmModeGetEncoder {
    pub encoder_id: u32,
    pub encoder_type: u32,

    pub crtc_id: u32,

    pub possible_crtcs: u32,
    pub possible_clones: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmModeGetConnector {
    pub encoders_ptr: u64,
    pub modes_ptr: u64,
    pub props_ptr: u64,
    pub prop_values_ptr: u64,

    pub count_modes: u32,
    pub count_props: u32,
    pub count_encoders: u32,

    pub encoder_id: u32,
    pub connector_id: u32,

    pub connector_type: u32,
    pub connector_type_id: u32,

    pub connection: u32,

    pub mm_width: u32,
    pub mm_height: u32,
    pub subpixel: u32,

    pub pad: u32,
}

impl DrmModeGetConnector {
    pub fn is_first_call(&self) -> bool {
        return self.encoders_ptr == 0
            && self.modes_ptr == 0
            && self.props_ptr == 0
            && self.prop_values_ptr == 0;
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmModeGetProperty {
    pub values_ptr: u64,
    pub enum_blob_ptr: u64,

    pub prop_id: u32,
    pub flags: u32,
    pub name: [u8; DRM_PROP_NAME_LEN],

    pub count_values: u32,
    pub count_enum_blobs: u32,
}

impl DrmModeGetProperty {
    pub fn is_first_call(&self) -> bool {
        return self.values_ptr == 0 && self.enum_blob_ptr == 0;
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmModeGetBlob {
    pub blob_id: u32,
    pub length: u32,
    pub data: u64,
}

impl DrmModeGetBlob {
    pub fn is_first_call(&self) -> bool {
        return self.length == 0;
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmModeFbCmd {
    pub fb_id: u32,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub bpp: u32,
    pub depth: u32,
    pub handle: u32,
}

bitflags::bitflags! {
    pub struct DrmModeFb: u32 {
        const INTERLACED = 1 << 0;
        const MODIFIERS = 1 << 1;
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmModeFbCmd2 {
    pub fb_id: u32,
    pub width: u32,
    pub height: u32,
    pub pixel_format: u32,
    pub flags: u32,
    pub handles: [u32; 4],
    pub pitches: [u32; 4],
    pub offsets: [u32; 4],
    __padding: u32,
    pub modifier: [u64; 4],
}

impl From<DrmModeFbCmd> for DrmModeFbCmd2 {
    fn from(fb_cmd: DrmModeFbCmd) -> Self {
        let pixel_format = match (fb_cmd.bpp, fb_cmd.depth) {
            (8, 8) => DrmFormat::C8 as u32,
            (16, 15) => DrmFormat::XRGB1555 as u32,
            (16, 16) => DrmFormat::RGB565 as u32,
            (24, 24) => DrmFormat::RGB888 as u32,
            (32, 24) => DrmFormat::XRGB8888 as u32,
            (32, 30) => DrmFormat::XRGB2101010 as u32,
            (32, 32) => DrmFormat::ARGB8888 as u32,
            _ => DrmFormat::Unknown as u32,
        };

        let mut handles = [0u32; 4];
        let mut pitches = [0u32; 4];
        let offsets = [0u32; 4];
        let modifier = [0u64; 4];

        handles[0] = fb_cmd.handle;
        pitches[0] = fb_cmd.pitch;

        Self {
            fb_id: fb_cmd.fb_id,
            width: fb_cmd.width,
            height: fb_cmd.height,
            pixel_format,
            flags: 0,
            handles,
            pitches,
            offsets,
            __padding: 0,
            modifier,
        }
    }
}

bitflags::bitflags! {
    pub struct PageFlipFlags: u32 {
        /// Request a flip complete event
        const EVENT = 0x01;
        /// Request async page flip (don’t wait for vblank)
        const ASYNC = 0x02;
        /// Absolute sequence target (optional)
        const TARGET_ABSOLUTE = 0x04;
        /// Relative sequence target (optional)
        const TARGET_RELATIVE = 0x08;

        /// Combined target mask
        const TARGET = Self::TARGET_ABSOLUTE.bits | Self::TARGET_RELATIVE.bits;
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmModeCrtcPageFlip {
    pub crtc_id: u32,
    pub fb_id: u32,
    pub flags: u32,
    pub reserved: u32,
    pub user_data: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmModeCrtcPageFlipTarget {
    pub crtc_id: u32,
    pub fb_id: u32,
    pub flags: u32,
    pub sequence: u32,
    pub user_data: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmModeFbDirtyCmd {
    pub fb_id: u32,
    pub flags: u32,
    pub color: u32,
    pub num_clips: u32,
    pub clips_ptr: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmModeCreateDumb {
    pub height: u32,
    pub width: u32,
    pub bpp: u32,
    pub flags: u32,
    pub handle: u32,
    pub pitch: u32,
    pub size: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmModeMapDumb {
    pub handle: u32,
    pub pad: u32,
    pub offset: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmModeDestroyDumb {
    pub handle: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmModeGetPlaneRes {
    pub plane_id_ptr: u64,
    pub count_planes: u32,
    __padding: u32,
}

impl DrmModeGetPlaneRes {
    pub fn is_first_call(&self) -> bool {
        return self.plane_id_ptr == 0;
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmModeGetPlane {
    pub plane_id: u32,
    pub crtc_id: u32,
    pub fb_id: u32,
    pub possible_crtcs: u32,
    /// Never used.
    pub gamma_size: u32,
    pub count_format_types: u32,
    pub format_type_ptr: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmModeObjectGetProps {
    pub props_ptr: u64,
    pub prop_values_ptr: u64,
    pub count_props: u32,
    pub obj_id: u32,
    pub obj_type: u32,
    _padding: u32,
}

impl DrmModeObjectGetProps {
    pub fn is_first_call(&self) -> bool {
        return self.props_ptr == 0 && self.prop_values_ptr == 0;
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmModeCreateBlob {
    pub data: u64,
    pub length: u32,
    pub blob_id: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmModeDestroyBlob {
    pub blob_id: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmSyncobjCreate {
    pub handle: u32,
    pub flags: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmSyncobjDestroy {
    pub handle: u32,
    pub pad: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmSyncobjWait {
    pub handles: u64,
    pub timeout_nsec: u64,
    pub count_handles: u32,
    pub flags: u32,
    pub first_signaled: u32,
    pub pad: u32,
    pub deadline_nsec: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmSyncobjArray {
    pub handles: u64,
    pub count_handles: u32,
    pub pad: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct VirtGpuExecBuffer {
    pub flags: u32,
    pub size: u32,
    pub command: u64,
    pub bo_handles: u64,
    pub num_bo_handles: u32,
    pub fence_fd: i32,
    pub ring_idx: u32,
    pub syncobj_stride: u32,
    pub num_in_syncobjs: u32,
    pub num_out_syncobjs: u32,
    pub in_syncobjs: u64,
    pub out_syncobjs: u64,
}