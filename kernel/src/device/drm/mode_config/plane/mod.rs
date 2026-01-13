use alloc::sync::Arc;

use hashbrown::HashMap;

use crate::{
    device::drm::mode_config::{DrmModeConfig, DrmModeObject, plane::funcs::PlaneFuncs},
    prelude::*,
};

pub mod funcs;
pub mod property;

#[repr(u64)]
#[derive(Debug, Clone, Copy)]
pub enum PlaneType {
    Overlay = 0,
    Primary = 1,
    Cursor = 2,
}

#[derive(Debug)]
pub struct DrmPlane {
    id: u32,
    type_: PlaneType,
    fb_id: u32,

    properties: HashMap<u32, u64>,
    funcs: Box<dyn PlaneFuncs>,
}

impl DrmPlane {
    pub fn init(res: &mut DrmModeConfig, type_: PlaneType, funcs: Box<dyn PlaneFuncs>) -> Result<Arc<Self>> {
        let id = res.next_object_id();
        let plane = Self {
            id,
            type_,
            fb_id: 0,
            properties: HashMap::new(),
            funcs,
        };

        let plane = Arc::new(plane);
        res.planes.insert(id, plane.clone());
        res.objects.insert(id, plane.clone());

        Ok(plane)
    }

    pub fn id(&self) -> u32 {
        self.id
    }
    pub fn type_(&self) -> PlaneType {
        self.type_
    }
    pub fn fb_id(&self) -> u32 {
        self.fb_id
    }
}

impl DrmModeObject for DrmPlane {
    fn id(&self) -> u32 {
        self.id
    }

    fn properties(&self) -> &HashMap<u32, u64> {
        &self.properties
    }
}
