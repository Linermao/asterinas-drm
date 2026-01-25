use alloc::{
    string::{String, ToString},
    sync::Arc,
};

use hashbrown::HashMap;

use crate::drm::driver::DrmDriver;

pub mod device;
pub mod driver;
pub mod gem;
pub mod mode_config;

#[derive(Debug, Default)]
pub(crate) struct DrmDrivers {
    drivers: HashMap<String, Arc<dyn DrmDriver>>,
}

impl DrmDrivers {
    pub fn new() -> Self {
        Self {
            drivers: HashMap::new(),
        }
    }

    /// Snapshot (clone Arcs) so caller can use it after unlocking the mutex.
    pub fn snapshot(&self) -> HashMap<String, Arc<dyn DrmDriver>> {
        self.drivers.clone()
    }

    pub fn register_driver(
        &mut self,
        name: &str,
        driver: Arc<dyn DrmDriver>,
    ) -> Result<(), super::Error> {
        self.drivers.insert(name.to_string(), driver);
        Ok(())
    }

    pub fn unregister_driver(&mut self, name: &str) -> Result<Arc<dyn DrmDriver>, super::Error> {
        if let Some(driver) = self.drivers.remove(name) {
            Ok(driver)
        } else {
            Err(super::Error::NotFound)
        }
    }
}
