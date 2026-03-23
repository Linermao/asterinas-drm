use alloc::sync::Arc;
use core::{any::Any, fmt::Debug};

use aster_time::read_monotonic_time;
use ostd::sync::Mutex;

use crate::drm::{
    DrmError,
    atomic::vblank::DrmVblankState,
    drm_modes::DrmDisplayMode,
    mode_config::ObjectId,
    mode_object::{DrmObject, DrmObjectCast, plane::DrmPlane, property::PropertyObject},
};

pub mod property;

#[derive(Debug)]
pub struct CrtcState {
    properties: PropertyObject,
    display_mode: Option<DrmDisplayMode>,
    gamma_size: u32,
    enable: bool,
    active: bool,
}

impl CrtcState {
    pub fn new(properties: PropertyObject) -> Self {
        Self {
            properties,
            display_mode: None,
            gamma_size: 0,
            enable: false,
            active: false,
        }
    }
}

pub trait DrmCrtc: Debug + Send + Sync + Any {
    fn state(&self) -> &Mutex<CrtcState>;
    fn vblank_state(&self) -> &Mutex<DrmVblankState>;
    fn primary_plane(&self) -> &Arc<dyn DrmPlane>;
    fn primary_plane_id(&self) -> ObjectId; // TODO
    fn cursor_plane(&self) -> &Option<Arc<dyn DrmPlane>>;
}

impl dyn DrmCrtc {
    pub fn gamma_size(&self) -> u32 {
        self.state().lock().gamma_size
    }

    pub fn enable(&self) -> bool {
        self.state().lock().enable
    }

    pub fn display_mode(&self) -> Option<DrmDisplayMode> {
        self.state().lock().display_mode
    }

    pub fn set_display_mode(&self, display_mode: DrmDisplayMode) {
        self.state().lock().display_mode = Some(display_mode);
    }

    pub fn count_props(&self) -> u32 {
        self.state().lock().properties.iter().count() as u32
    }

    pub fn get_properties(&self) -> PropertyObject {
        self.state().lock().properties.clone()
    }

    pub fn active(&self) -> bool {
        self.state().lock().active
    }

    pub fn set_active(&self, active: bool) {
        self.state().lock().active = active;
    }

    pub fn handle_vblank(&self) -> Result<(), DrmError> {
        let vblank_state = self.vblank_state();
        let vblank = vblank_state.lock();

        // 1. Increment vblank counter
        let sequence = vblank.increment();

        // 2. Update timestamp
        let timestamp = read_monotonic_time(); // TODO: Get actual monotonic time
        vblank.update_time(timestamp);

        // 3. Take all pending events
        let pending_events = vblank.take_pending_events();

        drop(vblank);

        // 4. Send each event
        for event in pending_events {
            event.send(
                sequence,
                timestamp.as_secs() as u32,
                timestamp.subsec_micros(),
            );
        }

        Ok(())
    }
}

impl DrmObjectCast for dyn DrmCrtc {
    fn cast(obj: &DrmObject) -> Option<&Arc<Self>> {
        if let DrmObject::Crtc(c) = obj {
            Some(c)
        } else {
            None
        }
    }
}
