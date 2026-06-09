// SPDX-License-Identifier: MPL-2.0

use alloc::vec::Vec;
use core::fmt::Debug;

use ostd::sync::Mutex;

use crate::{
    display::{DrmDisplayInfo, DrmDisplayMode},
    kms::object::{DrmKmsObject, DrmKmsObjectCast, KmsObjectId, KmsObjectIndex},
};

#[repr(u32)]
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum DrmConnType {
    UNKNOWN = 0,
    VGA = 1,
    DVII = 2,
    DVID = 3,
    DVIA = 4,
    COMPOSITE = 5,
    SVIDEO = 6,
    LVDS = 7,
    COMPONENT = 8,
    _9PINDIN = 9,
    DISPLAYPORT = 10,
    HDMIA = 11,
    HDMIB = 12,
    TV = 13,
    EDP = 14,
    VIRTUAL = 15,
    DSI = 16,
    DPI = 17,
    WRITEBACK = 18,
    SPI = 19,
    USB = 20,
}

#[repr(u32)]
#[derive(Debug, Clone, Copy)]
pub enum DrmConnStatus {
    // DRM_MODE_CONNECTED
    Connected = 1,
    // DRM_MODE_DISCONNECTED
    Disconnected = 2,
    // DRM_MODE_UNKNOWNCONNECTION
    Unknownconnection = 3,
}

impl Default for DrmConnStatus {
    fn default() -> Self {
        Self::Unknownconnection
    }
}

#[derive(Debug, Default)]
pub struct DrmConnState {
    status: DrmConnStatus,
    display_modes: Vec<DrmDisplayMode>,
    display_info: DrmDisplayInfo,
    encoder_id: Option<KmsObjectId>,
}

impl DrmConnState {
    pub fn snapshot(&self) -> DrmConnectorSnapshot {
        DrmConnectorSnapshot {
            status: self.status,
            display_modes: self.display_modes.clone(),
            mm_width: self.display_info.mm_width(),
            mm_height: self.display_info.mm_height(),
            subpixel: self.display_info.subpixel_order(),
            encoder_id: self.encoder_id,
        }
    }

    pub fn set_current_encoder_id(&mut self, encoder_id: Option<KmsObjectId>) {
        self.encoder_id = encoder_id;
    }

    pub fn set_display_state(
        &mut self,
        status: DrmConnStatus,
        display_modes: Vec<DrmDisplayMode>,
        display_info: DrmDisplayInfo,
        encoder_id: Option<KmsObjectId>,
    ) {
        self.status = status;
        self.display_modes = display_modes;
        self.display_info = display_info;
        self.encoder_id = encoder_id;
    }
}

#[derive(Debug, Clone)]
pub struct DrmConnectorSnapshot {
    status: DrmConnStatus,
    display_modes: Vec<DrmDisplayMode>,
    mm_width: u32,
    mm_height: u32,
    subpixel: u32,
    encoder_id: Option<KmsObjectId>,
}

impl DrmConnectorSnapshot {
    pub fn status(&self) -> DrmConnStatus {
        self.status
    }

    pub fn display_modes(&self) -> &[DrmDisplayMode] {
        &self.display_modes
    }

    pub fn mm_width(&self) -> u32 {
        self.mm_width
    }

    pub fn mm_height(&self) -> u32 {
        self.mm_height
    }

    pub fn subpixel(&self) -> u32 {
        self.subpixel
    }

    pub fn encoder_id(&self) -> Option<KmsObjectId> {
        self.encoder_id
    }
}

#[derive(Debug)]
pub struct DrmConnector {
    type_: DrmConnType,
    type_index: u32,
    state: Mutex<DrmConnState>,
    possible_encoders: u32,
}

impl DrmConnector {
    pub fn new(type_: DrmConnType, type_index: u32, possible_encoders: &[KmsObjectIndex]) -> Self {
        let mut possible_encoders_mask = 0;
        for &index in possible_encoders {
            possible_encoders_mask |= 1 << index;
        }

        Self {
            type_,
            type_index,
            state: Mutex::new(DrmConnState::default()),
            possible_encoders: possible_encoders_mask,
        }
    }

    pub fn type_(&self) -> DrmConnType {
        self.type_
    }

    pub fn type_index(&self) -> u32 {
        self.type_index
    }

    pub fn state(&self) -> &Mutex<DrmConnState> {
        &self.state
    }

    pub fn possible_encoders(&self) -> u32 {
        self.possible_encoders
    }

    pub fn state_snapshot(&self) -> DrmConnectorSnapshot {
        self.state.lock().snapshot()
    }
}

impl DrmKmsObjectCast for DrmConnector {
    fn cast(obj: &DrmKmsObject) -> Option<&Self> {
        if let DrmKmsObject::Connector(connector) = obj {
            Some(connector)
        } else {
            None
        }
    }
}
