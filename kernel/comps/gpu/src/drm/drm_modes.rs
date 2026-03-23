use alloc::format;

use int_to_c_enum::TryFromInt;

use crate::drm::DrmError;

const DRM_DISPLAY_MODE_LEN: usize = 32;

bitflags::bitflags! {
    pub struct DrmModeFlag: u32 {
        const PHSYNC    = 1 << 0;
        const NHSYNC    = 1 << 1;
        const PVSYNC    = 1 << 2;
        const NVSYNC    = 1 << 3;
        const INTERLACE = 1 << 4;
        const DBLSCAN   = 1 << 5;
        const CSYNC     = 1 << 6;
        const PCSYNC    = 1 << 7;
        const NCSYNC    = 1 << 8;
        const HSKEW     = 1 << 9;
        const BCAST     = 1 << 10; // deprecated
        const PIXMUX    = 1 << 11; // deprecated
        const DBLCLK    = 1 << 12;
        const CLKDIV2   = 1 << 13;
    }
}

bitflags::bitflags! {
    pub struct DrmModeType: u32 {
        const BUILTIN   = 1 << 0; /* deprecated */
        const CLOCK_C   = (1 << 1) | Self::BUILTIN.bits(); /* deprecated */
        const CRTC_C    = (1 << 2) | Self::BUILTIN.bits(); /* deprecated */
        const PREFERRED = 1 << 3;
        const DEFAULT   = 1 << 4; /* deprecated */
        const USERDEF   = 1 << 5;
        const DRIVER    = 1 << 6;
    }
}

const fn fourcc_code(a: u8, b: u8, c: u8, d: u8) -> u32 {
    (a as u32) | ((b as u32) << 8) | ((c as u32) << 16) | ((d as u32) << 24)
}

#[repr(u32)]
#[derive(Debug, Clone, Copy)]
pub enum DrmFormat {
    XRGB8888 = fourcc_code(b'X', b'R', b'2', b'4'),
    ARGB8888 = fourcc_code(b'A', b'R', b'2', b'4'),
    XBGR8888 = fourcc_code(b'X', b'B', b'2', b'4'),
    RGBX8888 = fourcc_code(b'R', b'X', b'2', b'4'),
    BGRX8888 = fourcc_code(b'B', b'X', b'2', b'4'),
    C8 = fourcc_code(b'C', b'8', b' ', b' '),
    XRGB1555 = fourcc_code(b'X', b'R', b'1', b'5'),
    RGB565 = fourcc_code(b'R', b'G', b'1', b'6'),
    RGB888 = fourcc_code(b'R', b'G', b'2', b'4'),
    XRGB2101010 = fourcc_code(b'X', b'R', b'3', b'0'),
    Unknown = fourcc_code(b' ', b' ', b' ', b' '),
}

#[derive(Debug, Clone, Copy)]
pub struct DrmDisplayMode {
    clock: u32,
    hdisplay: u16,
    hsync_start: u16,
    hsync_end: u16,
    htotal: u16,
    hskew: u16,
    vdisplay: u16,
    vsync_start: u16,
    vsync_end: u16,
    vtotal: u16,
    vscan: u16,

    flags: u32,
    type_: u32,

    name: [u8; DRM_DISPLAY_MODE_LEN],
}

impl Default for DrmDisplayMode {
    fn default() -> Self {
        let mut name = [0u8; 32];
        let bytes = "1280x800".as_bytes();
        let len = bytes.len().min(32);
        name[..len].copy_from_slice(&bytes[..len]);

        Self {
            clock: 83500, // kHz

            hdisplay: 1280,
            hsync_start: 1328,
            hsync_end: 1360,
            htotal: 1440,

            hskew: 0,

            vdisplay: 800,
            vsync_start: 803,
            vsync_end: 809,
            vtotal: 823,

            vscan: 0,

            flags: (DrmModeFlag::PHSYNC | DrmModeFlag::NVSYNC).bits(),
            type_: (DrmModeType::DRIVER | DrmModeType::PREFERRED).bits(),

            name,
        }
    }
}

impl DrmDisplayMode {
    /// Create a display mode from resolution (width x height).
    /// This creates a simple mode with reasonable timing values for the given resolution.
    pub fn from_resolution(width: u16, height: u16) -> Self {
        let w = width as u32;
        let h = height as u32;

        let h_blank = (w / 5).max(80); // >=20%
        let h_fp = (h_blank / 4).max(8);
        let h_sync = (h_blank / 4).max(32);
        let h_bp = h_blank.saturating_sub(h_fp + h_sync).max(24);

        let v_blank = (h / 20).max(10); // >=5%
        let v_fp = 1u32;
        let v_sync = 3u32;
        let v_bp = v_blank.saturating_sub(v_fp + v_sync).max(6);

        let hdisplay = w;
        let hsync_start = hdisplay + h_fp;
        let hsync_end = hsync_start + h_sync;
        let htotal = hsync_end + h_bp;

        let vdisplay = h;
        let vsync_start = vdisplay + v_fp;
        let vsync_end = vsync_start + v_sync;
        let vtotal = vsync_end + v_bp;

        let clock = ((htotal * vtotal * 60) + 500) / 1000; // kHz, rounded

        let mut name = [0u8; DRM_DISPLAY_MODE_LEN];
        let s = alloc::format!("{width}x{height}");
        let n = s.as_bytes();
        let len = n.len().min(DRM_DISPLAY_MODE_LEN - 1);
        name[..len].copy_from_slice(&n[..len]);

        Self {
            clock,
            hdisplay: hdisplay as u16,
            hsync_start: hsync_start as u16,
            hsync_end: hsync_end as u16,
            htotal: htotal as u16,
            hskew: 0,
            vdisplay: vdisplay as u16,
            vsync_start: vsync_start as u16,
            vsync_end: vsync_end as u16,
            vtotal: vtotal as u16,
            vscan: 0,
            flags: (DrmModeFlag::PHSYNC | DrmModeFlag::PVSYNC).bits(),
            type_: (DrmModeType::DRIVER | DrmModeType::PREFERRED).bits(),
            name,
        }
    }

    fn vrefresh(&self) -> u32 {
        if self.htotal == 0 || self.vtotal == 0 {
            return 0;
        }

        let mut num = self.clock as u64;
        let mut den = (self.htotal as u64) * (self.vtotal as u64);

        let flags = DrmModeFlag::from_bits_truncate(self.flags);

        if flags.contains(DrmModeFlag::INTERLACE) {
            num *= 2;
        }

        if flags.contains(DrmModeFlag::DBLSCAN) {
            den *= 2;
        }

        if self.vscan > 1 {
            den *= self.vscan as u64;
        }

        ((num * 1000 + den / 2) / den) as u32
    }
}

impl Into<DrmModeModeInfo> for DrmDisplayMode {
    fn into(self) -> DrmModeModeInfo {
        DrmModeModeInfo {
            clock: self.clock,
            hdisplay: self.hdisplay,
            hsync_start: self.hsync_start,
            hsync_end: self.hsync_end,
            htotal: self.htotal,
            hskew: self.hskew,
            vdisplay: self.vdisplay,
            vsync_start: self.vsync_start,
            vsync_end: self.vsync_end,
            vtotal: self.vtotal,
            vscan: self.vscan,
            vrefresh: self.vrefresh(),
            flags: self.flags,
            type_: self.type_,
            name: self.name,
        }
    }
}

bitflags::bitflags! {
    pub struct SubpixelOrder: u32 {
        const RGB444    = 1<<0;
        const YCBCR444  = 1<<1;
        const YCBCR422  = 1<<2;
        const YCBCR420  = 1<<3;
    }
}

#[derive(Debug)]
pub struct DrmDisplayInfo {
    mm_width: u32,
    mm_height: u32,
    subpixel_order: SubpixelOrder,
}

impl Default for DrmDisplayInfo {
    fn default() -> Self {
        Self {
            mm_width: 340,
            mm_height: 190,
            subpixel_order: SubpixelOrder::RGB444,
        }
    }
}

impl DrmDisplayInfo {
    pub fn new(mm_width: u32, mm_height: u32, subpixel_order: SubpixelOrder) -> Self {
        Self {
            mm_width,
            mm_height,
            subpixel_order,
        }
    }

    pub fn mm_width(&self) -> u32 {
        self.mm_width
    }

    pub fn mm_height(&self) -> u32 {
        self.mm_height
    }

    pub fn subpixel_order(&self) -> u32 {
        self.subpixel_order.bits()
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct DrmModeModeInfo {
    pub clock: u32,
    pub hdisplay: u16,
    pub hsync_start: u16,
    pub hsync_end: u16,
    pub htotal: u16,
    pub hskew: u16,
    pub vdisplay: u16,
    pub vsync_start: u16,
    pub vsync_end: u16,
    pub vtotal: u16,
    pub vscan: u16,

    pub vrefresh: u32,

    pub flags: u32,
    pub type_: u32,

    pub name: [u8; DRM_DISPLAY_MODE_LEN],
}

impl Into<DrmDisplayMode> for DrmModeModeInfo {
    fn into(self) -> DrmDisplayMode {
        DrmDisplayMode {
            clock: self.clock,
            hdisplay: self.hdisplay,
            hsync_start: self.hsync_start,
            hsync_end: self.hsync_end,
            htotal: self.htotal,
            hskew: self.hskew,
            vdisplay: self.vdisplay,
            vsync_start: self.vsync_start,
            vsync_end: self.vsync_end,
            vtotal: self.vtotal,
            vscan: self.vscan,
            flags: self.flags,
            type_: self.type_,
            name: self.name,
        }
    }
}
