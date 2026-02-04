use alloc::{boxed::Box, sync::Arc};

use hashbrown::{HashMap, HashSet};

use crate::drm::{
    DrmError,
    mode_config::{
        DrmModeConfig, DrmModeModeInfo, DrmModeObject, connector::funcs::ConnectorFuncs,
        encoder::DrmEncoder,
    },
};

pub mod funcs;
pub mod property;

#[repr(u32)]
#[derive(Debug, Clone, Copy)]
pub enum DrmModeConnType {
    Unknown = 0,
    VGA = 1,
    DVII = 2,
    DVID = 3,
    DVIA = 4,
    Composite = 5,
    SVIDEO = 6,
    LVDS = 7,
    Component = 8,
    _9PinDIN = 9,
    DisplayPort = 10,
    HDMIA = 11,
    HDMIB = 12,
    TV = 13,
    eDP = 14,
    VIRTUAL = 15,
    DSI = 16,
    DPI = 17,
    WRITEBACK = 18,
    SPI = 19,
    USB = 20,
}

#[repr(u32)]
#[derive(Debug, Clone, Copy)]
pub enum ConnectorStatus {
    // DRM_MODE_CONNECTED
    Connected = 1,
    // DRM_MODE_DISCONNECTED
    Disconnected = 2,
    // DRM_MODE_UNKNOWNCONNECTION
    Unknownconnection = 3,
}

bitflags::bitflags! {
    struct SubpixelOrder: u32 {
        const RGB444    = 1<<0;
        const YCBCR444  = 1<<1;
        const YCBCR422  = 1<<2;
        const YCBCR420  = 1<<3;
    }
}

#[derive(Debug)]
struct DrmDisplayInfo {
    mm_width: u32,
    mm_height: u32,
    subpixel_order: SubpixelOrder,
}

impl DrmDisplayInfo {
    pub fn mm_width(&self) -> u32 {
        self.mm_width
    }

    pub fn mm_height(&self) -> u32 {
        self.mm_height
    }

    pub fn subpixel_order(&self) -> u32 {
        self.subpixel_order.bits()
    }
}

#[derive(Debug)]
pub struct DrmConnector {
    id: u32,
    encoder: Option<u32>,
    modes: HashSet<DrmModeModeInfo>,
    properties: HashMap<u32, u64>,
    possible_encoders_id: HashSet<u32>,
    possible_encoders_mask: u32,

    type_: DrmModeConnType,
    type_id: u32,
    status: ConnectorStatus,

    display_info: DrmDisplayInfo,
    funcs: Box<dyn ConnectorFuncs>,
}

impl DrmConnector {
    pub fn init_with_encoder(
        res: &mut DrmModeConfig,
        status: ConnectorStatus,
        mode: &[DrmModeModeInfo],
        encoder: &[Arc<DrmEncoder>],
        funcs: Box<dyn ConnectorFuncs>,
    ) -> Result<Arc<Self>, DrmError> {
        let id = res.next_object_id();
        let mut conn = Self {
            id,
            encoder: None,
            modes: HashSet::new(),
            properties: HashMap::new(),
            possible_encoders_id: HashSet::new(),
            possible_encoders_mask: 0,

            type_: DrmModeConnType::Unknown,
            // TODO: auto allocat, not repeat
            type_id: 1,
            status,

            // TODO: use true data
            display_info: DrmDisplayInfo {
                mm_width: 384,
                mm_height: 240,
                subpixel_order: SubpixelOrder { bits: 0 },
            },
            funcs,
        };

        mode.iter().for_each(|m| {
            conn.modes.insert(*m);
        });

        encoder.iter().for_each(|e| {
            conn.possible_encoders_id.insert(e.id());
            conn.possible_encoders_mask |= 1u32 << e.index();
        });

        let conn = Arc::new(conn);
        res.connectors.insert(id, conn.clone());
        res.objects.insert(id, conn.clone());

        Ok(conn)
    }

    pub fn attach_property(&mut self, property_id: u32, value: u64) {
        self.properties.insert(property_id, value);
    }

    pub fn type_(&self) -> DrmModeConnType {
        self.type_
    }

    pub fn type_id_(&self) -> u32 {
        self.type_id
    }

    pub fn status(&self) -> ConnectorStatus {
        self.status
    }

    pub fn mm_width(&self) -> u32 {
        self.display_info.mm_width()
    }

    pub fn mm_height(&self) -> u32 {
        self.display_info.mm_height()
    }

    pub fn subpixel_order(&self) -> u32 {
        self.display_info.subpixel_order()
    }

    pub fn encoder(&self) -> Option<u32> {
        self.encoder
    }

    pub fn modes(&self) -> impl Iterator<Item = &DrmModeModeInfo> {
        self.modes.iter()
    }

    pub fn properties(&self) -> impl Iterator<Item = (&u32, &u64)> {
        self.properties.iter()
    }

    pub fn possible_encoders_id(&self) -> impl Iterator<Item = &u32> {
        self.possible_encoders_id.iter()
    }

    pub fn count_modes(&self) -> u32 {
        self.modes.iter().count() as u32
    }

    pub fn count_props(&self) -> u32 {
        self.properties.iter().count() as u32
    }

    pub fn count_encoders(&self) -> u32 {
        self.possible_encoders_mask.count_ones()
    }
}

impl DrmModeObject for DrmConnector {
    fn id(&self) -> u32 {
        self.id
    }

    fn properties(&self) -> &HashMap<u32, u64> {
        &self.properties
    }
}
