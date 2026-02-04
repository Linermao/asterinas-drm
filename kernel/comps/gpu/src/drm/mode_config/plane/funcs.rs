use core::{any::Any, fmt::Debug};

use crate::drm::DrmError;

// TODO
pub trait PlaneFuncs: Debug + Any + Sync + Send {
    // fn update_plane(&self) -> Result<(), DrmError>;

    // fn disable_plane(&self) -> Result<(), DrmError>;

    // fn destroy(&self);

    // fn reset(&self);

    // fn set_property(&self) -> Result<(), DrmError>;

    // fn state(&self) -> Result<DrmPlaneState, DrmError>;

    // fn atomic_destroy_state(&self);

    // fn atomic_set_property(&self) -> Result<(), DrmError>;

    // fn atomic_get_property(&self) -> Result<(), DrmError>;

    // fn atomic_print_state(&self);

    // fn late_register(&self) -> Result<(), DrmError>;

    // fn early_unregister(&self);

    // fn format_mod_supported(&self) -> bool;

    // fn format_mod_supported_async(&self) -> bool;
}
