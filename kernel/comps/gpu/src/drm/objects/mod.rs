use alloc::{sync::Arc, vec, vec::Vec};
use core::{
    fmt::Debug,
    sync::atomic::{AtomicU32, Ordering},
};

use hashbrown::{HashMap, hash_map::Entry};

use crate::drm::{
    DrmError,
    objects::{
        connector::{ConnectorType, DrmConnector},
        crtc::DrmCrtc,
        encoder::DrmEncoder,
        framebuffer::DrmFramebuffer,
        plane::DrmPlane,
        property::{DrmModeBlob, DrmProperty, PropertyObject, PropertySpec},
    },
};

pub mod connector;
pub mod crtc;
pub mod encoder;
pub mod framebuffer;
pub mod plane;
pub mod property;

pub type ObjectId = u32;

#[derive(Debug)]
pub struct DrmObjects {
    plane_ids: Vec<ObjectId>,
    crtc_ids: Vec<ObjectId>,
    encoder_ids: Vec<ObjectId>,
    connector_ids: Vec<ObjectId>,

    object_by_id: HashMap<ObjectId, DrmObject>,
    next_object_id: AtomicU32,

    connector_count_by_type: HashMap<ConnectorType, u32>,
}

impl DrmObjects {
    pub fn new() -> Self {
        Self {
            plane_ids: Vec::new(),
            crtc_ids: Vec::new(),
            encoder_ids: Vec::new(),
            connector_ids: Vec::new(),

            object_by_id: HashMap::new(),
            next_object_id: AtomicU32::new(1),

            connector_count_by_type: HashMap::new(),
        }
    }

    pub fn next_object_id(&self) -> ObjectId {
        self.next_object_id.fetch_add(1, Ordering::SeqCst)
    }

    pub fn next_connector_type_id(&mut self, connector_type: ConnectorType) -> u32 {
        let count = self
            .connector_count_by_type
            .entry(connector_type)
            .or_insert(0);
        let type_id = *count;
        *count += 1;
        type_id
    }

    pub fn add_object(&mut self, object: DrmObject) -> (ObjectId, usize) {
        let id = self.next_object_id();

        let index = match object {
            DrmObject::Plane(_) => {
                let idx = self.plane_ids.len();
                self.plane_ids.push(id);
                idx
            }
            DrmObject::Crtc(_) => {
                let idx = self.crtc_ids.len();
                self.crtc_ids.push(id);
                idx
            }
            DrmObject::Encoder(_) => {
                let idx = self.encoder_ids.len();
                self.encoder_ids.push(id);
                idx
            }
            DrmObject::Connector(_) => {
                let idx = self.connector_ids.len();
                self.connector_ids.push(id);
                idx
            }
            _ => 0,
        };

        self.object_by_id.insert(id, object);

        (id, index)
    }

    pub fn add_blob(&mut self, data: Vec<u8>) -> ObjectId {
        let blob = Arc::new(DrmModeBlob::new(data));
        self.add_object(DrmObject::Blob(blob)).0
    }

    pub fn add_framebuffer(&mut self, fb: Arc<dyn DrmFramebuffer>) -> ObjectId {
        self.add_object(DrmObject::Framebuffer(fb)).0
    }

    pub fn count_objects(&self, type_: DrmObjectType) -> usize {
        match type_ {
            DrmObjectType::Crtc => self.crtc_ids.len(),
            DrmObjectType::Connector => self.connector_ids.len(),
            DrmObjectType::Encoder => self.encoder_ids.len(),
            DrmObjectType::Framebuffer => 0,
            DrmObjectType::Plane => self.plane_ids.len(),

            _ => 0,
        }
    }

    pub fn attach_property(&mut self, property_spec: &dyn PropertySpec) -> ObjectId {
        let property = Arc::new(property_spec.build());
        self.add_object(DrmObject::Property(property)).0
    }

    pub fn collect_object_ids(
        &self,
        type_: DrmObjectType,
        mask: Option<ObjectId>,
    ) -> Vec<ObjectId> {
        let res = match type_ {
            DrmObjectType::Crtc => &self.crtc_ids,
            DrmObjectType::Connector => &self.connector_ids,
            DrmObjectType::Encoder => &self.encoder_ids,
            DrmObjectType::Plane => &self.plane_ids,
            DrmObjectType::Framebuffer => &vec![],
            _ => &vec![],
        };

        res.iter()
            .enumerate()
            .filter(move |(i, _)| match mask {
                None => true,
                Some(m) => (m & (1 << i)) != 0,
            })
            .map(|(_, id)| *id)
            .collect()
    }

    pub fn get_object_by_id<T: DrmObjectCast + ?Sized>(&self, id: ObjectId) -> Option<Arc<T>> {
        let obj = self.object_by_id.get(&id)?;
        T::cast(obj).cloned()
    }

    pub fn get_object(&self, id: ObjectId, type_: DrmObjectType) -> Option<&DrmObject> {
        self.object_by_id
            .get(&id)
            .filter(|obj| type_ == DrmObjectType::Any || obj.type_() == type_)
    }

    pub fn remove_object_with<T: DrmObjectCast + ?Sized>(
        &mut self,
        id: ObjectId,
    ) -> Option<Arc<T>> {
        match self.object_by_id.entry(id) {
            Entry::Occupied(entry) => {
                let obj = entry.get();

                if let Some(casted) = T::cast(obj).cloned() {
                    let _ = entry.remove();
                    Some(casted)
                } else {
                    None
                }
            }
            Entry::Vacant(_) => None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum DrmObject {
    Plane(Arc<dyn DrmPlane>),
    Crtc(Arc<dyn DrmCrtc>),
    Encoder(Arc<dyn DrmEncoder>),
    Connector(Arc<dyn DrmConnector>),
    Framebuffer(Arc<dyn DrmFramebuffer>),
    Property(Arc<DrmProperty>),
    Blob(Arc<DrmModeBlob>),
}

pub trait DrmObjectCast {
    fn cast(obj: &DrmObject) -> Option<&Arc<Self>>;
}

impl DrmObject {
    pub fn type_(&self) -> DrmObjectType {
        match self {
            DrmObject::Plane(_) => DrmObjectType::Plane,
            DrmObject::Crtc(_) => DrmObjectType::Crtc,
            DrmObject::Encoder(_) => DrmObjectType::Encoder,
            DrmObject::Connector(_) => DrmObjectType::Connector,
            DrmObject::Framebuffer(_) => DrmObjectType::Framebuffer,
            DrmObject::Property(_) => DrmObjectType::Property,
            DrmObject::Blob(_) => DrmObjectType::Blob,
        }
    }

    pub fn count_props(&self) -> u32 {
        match self {
            DrmObject::Plane(plane) => plane.count_props(),
            DrmObject::Crtc(crtc) => crtc.count_props(),
            DrmObject::Connector(connector) => connector.count_props(),
            _ => 0,
        }
    }

    pub fn get_properties(&self) -> PropertyObject {
        match self {
            DrmObject::Plane(plane) => plane.get_properties(),
            DrmObject::Crtc(crtc) => crtc.get_properties(),
            DrmObject::Connector(connector) => connector.get_properties(),
            _ => HashMap::new(),
        }
    }
}

#[repr(u32)]
#[derive(Debug, PartialEq, Eq)]
pub enum DrmObjectType {
    Any = 0,
    Crtc = 0xCCCC_CCCC,
    Connector = 0xC0C0_C0C0,
    Encoder = 0xE0E0_E0E0,
    Mode = 0xDEDE_DEDE,
    Property = 0xB0B0_B0B0,
    Framebuffer = 0xFBFB_FBFB,
    Blob = 0xBBBB_BBBB,
    Plane = 0xEEEE_EEEE,
}

impl TryFrom<u32> for DrmObjectType {
    type Error = DrmError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Any),
            0xCCCC_CCCC => Ok(Self::Crtc),
            0xC0C0_C0C0 => Ok(Self::Connector),
            0xE0E0_E0E0 => Ok(Self::Encoder),
            0xDEDE_DEDE => Ok(Self::Mode),
            0xB0B0_B0B0 => Ok(Self::Property),
            0xFBFB_FBFB => Ok(Self::Framebuffer),
            0xBBBB_BBBB => Ok(Self::Blob),
            0xEEEE_EEEE => Ok(Self::Plane),
            _ => Err(DrmError::Invalid),
        }
    }
}
