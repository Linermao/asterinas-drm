use alloc::{sync::Arc, vec, vec::Vec};
use core::sync::atomic::{AtomicU32, Ordering};

use hashbrown::HashMap;

use crate::drm::mode_object::{
    DrmObject, DrmObjectCast, DrmObjectType, connector::ConnectorType, property::{DrmModeBlob, PropertySpec},
};

type ObjectId = u32;

#[derive(Debug)]
pub struct DrmModeConfig {
    planes: Vec<ObjectId>,
    crtcs: Vec<ObjectId>,
    encoders: Vec<ObjectId>,
    connectors: Vec<ObjectId>,

    objects: HashMap<ObjectId, DrmObject>,
    next_object_id: AtomicU32,

    connector_type_counts: HashMap<ConnectorType, u32>,

    min_width: u32,
    max_width: u32,
    min_height: u32,
    max_height: u32,
}

impl DrmModeConfig {
    pub fn new(min_width: u32, max_width: u32, min_height: u32, max_height: u32) -> Self {
        Self {
            planes: Vec::new(),
            crtcs: Vec::new(),
            encoders: Vec::new(),
            connectors: Vec::new(),

            objects: HashMap::new(),
            next_object_id: AtomicU32::new(1),

            connector_type_counts: HashMap::new(),
            min_width,
            max_width,
            min_height,
            max_height,
        }
    }

    pub fn next_object_id(&self) -> ObjectId {
        self.next_object_id.fetch_add(1, Ordering::SeqCst)
    }

    pub fn next_connector_type_id(&mut self, connector_type: ConnectorType) -> u32 {
        let count = self
            .connector_type_counts
            .entry(connector_type)
            .or_insert(0);
        let type_id = *count;
        *count += 1;
        type_id
    }

    pub fn add_object(&mut self, object: DrmObject) -> usize {
        let id = self.next_object_id();

        let index_or_id = match object {
            DrmObject::Plane(_) => {
                let idx = self.planes.len();
                self.planes.push(id);
                idx
            }
            DrmObject::Crtc(_) => {
                let idx = self.crtcs.len();
                self.crtcs.push(id);
                idx
            }
            DrmObject::Encoder(_) => {
                let idx = self.encoders.len();
                self.encoders.push(id);
                idx
            }
            DrmObject::Connector(_) => {
                let idx = self.connectors.len();
                self.connectors.push(id);
                idx
            }
            _ => id as usize,
        };

        self.objects.insert(id, object);

        index_or_id
    }

    pub fn add_blob(&mut self, data: Vec<u8>) -> usize {
        let blob = Arc::new(DrmModeBlob::new(data));
        self.add_object(DrmObject::Blob(blob))
    }

    pub fn count_objects(&self, type_: DrmObjectType) -> usize {
        match type_ {
            DrmObjectType::Crtc => self.crtcs.len(),
            DrmObjectType::Connector => self.connectors.len(),
            DrmObjectType::Encoder => self.encoders.len(),
            DrmObjectType::Framebuffer => 0,
            DrmObjectType::Plane => self.planes.len(),

            _ => 0,
        }
    }

    pub fn attach_property(&mut self, property_spec: &dyn PropertySpec) -> u32 {
        let property = Arc::new(property_spec.build());
        self.add_object(DrmObject::Property(property)) as u32
    }

    pub fn get_object_ids(&self, type_: DrmObjectType, mask: Option<u32>) -> Vec<u32> {
        let res = match type_ {
            DrmObjectType::Crtc => &self.crtcs,
            DrmObjectType::Connector => &self.connectors,
            DrmObjectType::Encoder => &self.encoders,
            DrmObjectType::Plane => &self.planes,
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

    pub fn get_object_with<T: DrmObjectCast + ?Sized>(&self, id: u32) -> Option<Arc<T>> {
        let obj = self.objects.get(&id)?;
        T::cast(obj).cloned()
    }

    pub fn get_object(&self, id: u32, type_: DrmObjectType) -> Option<&DrmObject> {
        self.objects
            .get(&id)
            .filter(|obj| type_ == DrmObjectType::Any || obj.type_() == type_)
    }

    pub fn max_width(&self) -> u32 {
        self.max_width
    }
    pub fn min_width(&self) -> u32 {
        self.min_width
    }
    pub fn max_height(&self) -> u32 {
        self.max_height
    }
    pub fn min_height(&self) -> u32 {
        self.min_height
    }
}
