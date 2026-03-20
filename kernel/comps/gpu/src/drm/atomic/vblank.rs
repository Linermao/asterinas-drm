pub const DRM_EVENT_VBLANK_LEN: u32 = 32;

#[repr(u32)]
#[derive(Debug)]
pub enum DrmVblankEvent {
    Vblank = 0x01,
    FlipComplete = 0x02,
    CrtcSequence = 0x03,
}