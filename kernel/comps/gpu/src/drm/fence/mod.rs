use core::fmt::Debug;
use crate::drm::DrmError;

pub trait DrmFence: Debug + Send + Sync {
    fn is_signaled(&self) -> bool;
    fn wait(&self) -> Result<(), DrmError>;
    fn signal(&self);
    fn context(&self) -> u64;
    fn seqno(&self) -> u64;
}
