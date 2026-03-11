use alloc::format;

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

            flags: (DrmModeFlag::PHSYNC | DrmModeFlag::PVSYNC).bits(),  // DRM_MODE_FLAG_PHSYNC | DRM_MODE_FLAG_PVSYNC
            type_: DrmModeType::DRIVER.bits(),

            name,
        }
    }
}

impl DrmDisplayMode {
    /// Create a display mode from resolution (width x height).
    /// This creates a simple mode with reasonable timing values for the given resolution.
    pub fn from_resolution(width: u16, height: u16) -> Self {
        let mut name = [0u8; DRM_DISPLAY_MODE_LEN];
        let name_str = format!("{}x{}", width, height);
        let bytes = name_str.as_bytes();
        let len = bytes.len().min(DRM_DISPLAY_MODE_LEN);
        name[..len].copy_from_slice(&bytes[..len]);

        // Calculate reasonable timing values
        // H blanking is typically around 20-25% of active display
        let h_blank = width / 4;
        let hsync_start = width + h_blank / 2;
        let hsync_end = width + h_blank * 3 / 4;
        let htotal = width + h_blank;

        // V blanking is typically around 5-10% of active display
        let v_blank = height / 20 + 1;
        let vsync_start = height + v_blank / 2;
        let vsync_end = height + v_blank * 3 / 4;
        let vtotal = height + v_blank;

        // Calculate pixel clock (assume 60Hz refresh rate)
        // clock = htotal * vtotal * refresh_rate / 1000 (kHz)
        let clock = (htotal as u32 * vtotal as u32 * 60) / 1000;

        Self {
            clock,
            hdisplay: width,
            hsync_start: hsync_start as u16,
            hsync_end: hsync_end as u16,
            htotal: htotal as u16,
            hskew: 0,
            vdisplay: height,
            vsync_start: vsync_start as u16,
            vsync_end: vsync_end as u16,
            vtotal: vtotal as u16,
            vscan: 0,
            flags: (DrmModeFlag::PHSYNC | DrmModeFlag::PVSYNC).bits(),
            type_: DrmModeType::DRIVER.bits(),
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
