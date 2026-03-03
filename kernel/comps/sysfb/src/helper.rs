use alloc::sync::Arc;

use aster_gpu::drm::{
    DrmError,
    driver::MemfdBackendCreateFunc,
    gem::DrmGemObject,
    ioctl::DrmModeCreateDumb,
    mode_config::{DrmModeModeInfo, connector::DrmConnector},
};

pub fn drm_sysfb_gem_create(
    args: &mut DrmModeCreateDumb,
    memfd_object_create: MemfdBackendCreateFunc,
) -> Result<Arc<DrmGemObject>, DrmError> {
    let pitch = args.width * (args.bpp / 8);
    let size = (pitch * args.height) as u64;
    args.pitch = pitch;
    args.size = size;

    // TODO: handle the error
    let backend = memfd_object_create("simpledrm-dumb", size)?;
    let gem_object = DrmGemObject::new(size, pitch, backend);

    Ok(Arc::new(gem_object))
}

pub fn drm_sysfb_connector_helper_get_modes(connector: Arc<DrmConnector>) -> Result<(), DrmError> {
    let fake_modeinfo = fake_modeinfo();
    connector.update_modes(&[fake_modeinfo])
}

// Create a fake display mode for testing and bring-up purposes.
//
// This mode is not obtained from real hardware (e.g. EDID or firmware).
// It provides a minimal, hard-coded timing description that allows the
// DRM pipeline to be exercised during early development, testing, or
// virtualized environments (such as simpledrm, QEMU, or headless setups).
//
// The values are chosen to represent a common 1280x800@60Hz mode and are
// sufficient for validating mode-setting, atomic state transitions, and
// userspace interaction. Real drivers must replace this with modes derived
// from hardware capabilities or display discovery mechanisms.
fn fake_modeinfo() -> DrmModeModeInfo {
    let mut name = [0u8; 32];
    let bytes = "1280x800".as_bytes();
    let len = bytes.len().min(32);
    name[..len].copy_from_slice(&bytes[..len]);

    DrmModeModeInfo {
        clock: 65000, // kHz (65 MHz)

        hdisplay: 1280,
        hsync_start: 1048,
        hsync_end: 1184,
        htotal: 1344,

        hskew: 0,

        vdisplay: 800,
        vsync_start: 771,
        vsync_end: 777,
        vtotal: 806,

        vscan: 0,

        vrefresh: 60,

        flags: 0x5,  // DRM_MODE_FLAG_PHSYNC | DRM_MODE_FLAG_PVSYNC
        type_: 0x40, // DRM_MODE_TYPE_DRIVER (0x40) or DRIVER | PREFERRED (0x60)

        name,
    }
}
