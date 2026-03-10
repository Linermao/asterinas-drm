use crate::drm::mode_object::property::{DrmProperty, PropertySpec};

#[derive(Debug)]
pub enum CrtcProps {

}

impl PropertySpec for CrtcProps {
    fn build(&self) -> DrmProperty {
        todo!()
    }
}