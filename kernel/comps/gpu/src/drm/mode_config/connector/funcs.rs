use alloc::sync::Arc;
use core::{any::Any, fmt::Debug};

use crate::{
    GpuDevice,
    drm::{DrmError, mode_config::connector::DrmConnector},
};

// TODO
pub trait ConnectorFuncs: Debug + Any + Sync + Send {
    fn fill_modes(
        &self,
        max_x: u32,
        max_y: u32,
        connector: Arc<DrmConnector>,
    ) -> Result<(), DrmError>;

    fn detect(&self, force: bool, connector: Arc<DrmConnector>) -> Result<(), DrmError>;

    fn get_modes(&self, connector: Arc<DrmConnector>) -> Result<(), DrmError>;
}

pub fn drm_helper_probe_single_connector_modes(
    max_x: u32,
    max_y: u32,
    connector: Arc<DrmConnector>,
) -> Result<(), DrmError> {
    // TODO:
    connector.funcs.detect(false, connector.clone())?;
    connector.funcs.get_modes(connector.clone())
}
