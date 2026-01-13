use alloc::{
    boxed::Box,
    format,
    string::{String, ToString},
    sync::Arc,
};
use core::sync::atomic::Ordering;

use hashbrown::HashMap;

use crate::{
    device::drm::mode_config::{
        DrmModeConfig, DrmModeObject, crtc::funcs::CrtcFuncs, plane::DrmPlane,
    },
    prelude::*,
};

pub mod funcs;
pub mod property;

#[derive(Debug)]
pub struct DrmCrtc {
    id: u32,
    /// human readable name, can be overwritten by the driver
    name: String,

    index: u8,

    properties: HashMap<u32, u64>,
    gamma_size: u32,
    primary_plane: Arc<DrmPlane>,
    cursor_plane: Option<Arc<DrmPlane>>,

    enabled: bool,

    /// xy position on screen.
    x: u32,
    y: u32,

    funcs: Box<dyn CrtcFuncs>,
}

impl DrmCrtc {
    pub fn id(&self) -> u32 {
        self.id
    }

    pub fn index(&self) -> u8 {
        self.index
    }

    pub fn xy(&self) -> (u32, u32) {
        // TODO: use plane state x,y
        (self.x, self.y)
    }

    pub fn fb_id(&self) -> u32 {
        // TODO: fallback to self.fb_id
        self.primary_plane.fb_id()
    }

    pub fn gamma_size(&self) -> u32 {
        self.gamma_size
    }

    pub fn init_with_planes(
        res: &mut DrmModeConfig,
        name: Option<&str>,
        primary_plane: Arc<DrmPlane>,
        cursor_plane: Option<Arc<DrmPlane>>,
        funcs: Box<dyn CrtcFuncs>,
    ) -> Result<Arc<Self>> {
        let id = res.next_object_id();
        let name = match name {
            Some(name) => name.to_string(),
            None => format!("crtc-{}", id),
        };

        let crtc = Self {
            id,
            name,
            index: res.crtc_index.fetch_add(1, Ordering::SeqCst),
            properties: HashMap::new(),
            gamma_size: 0,
            primary_plane,
            cursor_plane,
            enabled: false,
            x: 0,
            y: 0,
            funcs,
        };

        // TODO: get x, y, gamma_size

        let crtc = Arc::new(crtc);
        res.crtcs.insert(id, crtc.clone());
        res.objects.insert(id, crtc.clone());

        Ok(crtc)
    }
}

impl DrmModeObject for DrmCrtc {
    fn id(&self) -> u32 {
        self.id
    }

    fn properties(&self) -> &HashMap<u32, u64> {
        &self.properties
    }
}
