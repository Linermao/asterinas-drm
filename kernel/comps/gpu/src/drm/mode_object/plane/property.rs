use crate::drm::mode_object::property::{DrmProperty, PropertySpec};

#[derive(Debug)]
pub enum PlaneProps {

}

impl PropertySpec for PlaneProps {
    fn build(&self) -> DrmProperty {
        todo!()
    }
}