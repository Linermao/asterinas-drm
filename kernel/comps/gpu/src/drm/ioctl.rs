use crate::drm::{drm_modes::DrmModeModeInfo, mode_object::property::DRM_PROP_NAME_LEN};

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
        return self.length == 0
    }
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