// SPDX-License-Identifier: MPL-2.0

use alloc::vec::Vec;

use aster_core::prelude::*;

use crate::{
    device::DrmDevice, kms::{
        DrmKmsOps, display_mode::DrmDisplayMode, objects::{
            DrmKmsObjectStore, DrmKmsObjectType, KmsObjectId, crtc::DrmCrtcState, property::{DrmStandardProperty, KmsObjectPropValue},
        },
    },
};

/// TODO: add doc comment
pub trait DrmAtomicOps: DrmDevice + DrmKmsOps {
    fn atomic_commit_tail(&self) -> Result<()>;

    fn atomic_flush(&self) -> Result<()>;
}

#[derive(Debug, Default)]
pub(crate) struct DrmAtomicState {
    crtc_state: AtomicCrtcState
}

#[derive(Clone, Debug, Default)]
struct AtomicCrtcState {
    display_mode: Option<DrmDisplayMode>,
    is_active: bool,
}


#[derive(Debug)]
pub(crate) struct DrmAtomicRequest {
    object_id: KmsObjectId,
    object_props: Vec<(KmsObjectId, KmsObjectPropValue)>,
}

impl DrmAtomicRequest {
    pub(crate) fn new(
        object_id: KmsObjectId,
        object_props: Vec<(KmsObjectId, KmsObjectPropValue)>,
    ) -> Self {
        Self {
            object_id,
            object_props,
        }
    }
}

pub(crate) fn atomic_check(
    object_store: &DrmKmsObjectStore,
    atomic_requests: &[DrmAtomicRequest],
) -> Result<()> {
    let mut atomic_state = DrmAtomicState::default();

    for request in atomic_requests {
        let object_type = object_store
            .lookup_any_object_type(request.object_id)
            .ok_or(Errno::ENOENT)?;
        let object_props = object_store
            .lookup_any_object_properties(request.object_id)?
            .ok_or_else(|| {
                Error::with_message(
                    Errno::ENOENT,
                    "the DRM object type does not match the object ID",
                )
            })?;

        for (prop_id, value) in &request.object_props {
            let property = object_store
                .lookup_property(*prop_id)
                .ok_or(Errno::ENOENT)?;
            object_props
                .attachments()
                .iter()
                .find(|entry| entry.property().id() == *prop_id)
                .ok_or_else(|| {
                    Error::with_message(
                        Errno::ENOENT,
                        "TODO: the property doesn't match the object property",
                    )
                })?;

            if property.is_immutable() {
                return_errno_with_message!(Errno::EINVAL, "TODO: the property is immutable");
            }

            let standard_property = object_store
                .standard_property(property.id())
                .ok_or(Errno::ENOENT)?;

            match standard_property {
                DrmStandardProperty::SrcX => {
                    


                },
                DrmStandardProperty::SrcY => todo!(),
                DrmStandardProperty::SrcW => todo!(),
                DrmStandardProperty::SrcH => todo!(),
                DrmStandardProperty::CrtcX => todo!(),
                DrmStandardProperty::CrtcY => todo!(),
                DrmStandardProperty::CrtcW => todo!(),
                DrmStandardProperty::CrtcH => todo!(),
                DrmStandardProperty::FbId => todo!(),
                DrmStandardProperty::CrtcId => todo!(),
                DrmStandardProperty::Active => {
                    atomic_state.crtc_state.is_active = *value != 0;
                },
                DrmStandardProperty::ModeId => {
                    // TODO
                },
                _ => return_errno_with_message!(Errno::EINVAL, "TODO: unreachable property xxx")
            }            
        }
    }

    let _ = atomic_state;

    Ok(())
}

pub(crate) fn atomic_commit() -> Result<()> {
    Ok(())
}
