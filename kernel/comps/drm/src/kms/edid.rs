// SPDX-License-Identifier: MPL-2.0

use alloc::vec::Vec;

use crate::{
    DrmError,
    kms::display::{
        DRM_DISPLAY_MODE_NAME_LEN, DrmDisplayInfo, DrmDisplayMode, DrmModeFlag, DrmModeModeInfo,
        DrmModeType, SubpixelOrder,
    },
};

const EDID_BLOCK_SIZE: usize = 128;
const EDID_MAX_SIZE: usize = 1024;
const EDID_HEADER: [u8; 8] = [0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00];
const EDID_VERSION_OFFSET: usize = 18;
const EDID_WIDTH_CM_OFFSET: usize = 21;
const EDID_HEIGHT_CM_OFFSET: usize = 22;
const EDID_EXTENSIONS_OFFSET: usize = 126;
const EDID_DETAILED_TIMING_OFFSET: usize = 54;
const EDID_DETAILED_TIMING_SIZE: usize = 18;
const EDID_DETAILED_TIMING_COUNT: usize = 4;

#[derive(Debug, Clone)]
pub struct DrmEdid {
    raw: Vec<u8>,
    display_info: DrmDisplayInfo,
    modes: Vec<DrmDisplayMode>,
}

impl DrmEdid {
    pub fn new(raw: &[u8]) -> Result<Self, DrmError> {
        let edid_size = validate_edid(raw)?;
        let raw = &raw[..edid_size];

        Ok(Self {
            raw: raw.to_vec(),
            display_info: parse_display_info(raw)?,
            modes: parse_modes(raw)?,
        })
    }

    pub fn raw(&self) -> &[u8] {
        &self.raw
    }

    pub fn display_info(&self) -> DrmDisplayInfo {
        self.display_info
    }

    pub fn modes(&self) -> &[DrmDisplayMode] {
        &self.modes
    }
}

fn validate_edid(raw: &[u8]) -> Result<usize, DrmError> {
    if raw.len() < EDID_BLOCK_SIZE || raw.len() > EDID_MAX_SIZE || raw.len() % EDID_BLOCK_SIZE != 0
    {
        return Err(DrmError::Invalid);
    }

    let base_block = &raw[..EDID_BLOCK_SIZE];
    if base_block[..EDID_HEADER.len()] != EDID_HEADER[..] {
        return Err(DrmError::Invalid);
    }

    if base_block[EDID_VERSION_OFFSET] != 1 {
        return Err(DrmError::Invalid);
    }

    let block_count = usize::from(base_block[EDID_EXTENSIONS_OFFSET])
        .checked_add(1)
        .ok_or(DrmError::Invalid)?;
    let expected_size = block_count
        .checked_mul(EDID_BLOCK_SIZE)
        .ok_or(DrmError::Invalid)?;
    if expected_size > raw.len() {
        return Err(DrmError::Invalid);
    }

    for block in raw[..expected_size].chunks_exact(EDID_BLOCK_SIZE) {
        validate_block_checksum(block)?;
    }

    Ok(expected_size)
}

fn validate_block_checksum(block: &[u8]) -> Result<(), DrmError> {
    let checksum = block
        .iter()
        .fold(0u8, |checksum, byte| checksum.wrapping_add(*byte));
    if checksum == 0 {
        Ok(())
    } else {
        Err(DrmError::Invalid)
    }
}

fn parse_display_info(raw: &[u8]) -> Result<DrmDisplayInfo, DrmError> {
    let base_block = &raw[..EDID_BLOCK_SIZE];
    let mm_width = u32::from(base_block[EDID_WIDTH_CM_OFFSET])
        .checked_mul(10)
        .ok_or(DrmError::Invalid)?;
    let mm_height = u32::from(base_block[EDID_HEIGHT_CM_OFFSET])
        .checked_mul(10)
        .ok_or(DrmError::Invalid)?;

    Ok(DrmDisplayInfo::new(
        mm_width,
        mm_height,
        SubpixelOrder::Unknown,
    ))
}

fn parse_modes(raw: &[u8]) -> Result<Vec<DrmDisplayMode>, DrmError> {
    let base_block = &raw[..EDID_BLOCK_SIZE];
    let mut modes = Vec::new();

    for index in 0..EDID_DETAILED_TIMING_COUNT {
        let offset = EDID_DETAILED_TIMING_OFFSET
            .checked_add(
                index
                    .checked_mul(EDID_DETAILED_TIMING_SIZE)
                    .ok_or(DrmError::Invalid)?,
            )
            .ok_or(DrmError::Invalid)?;
        let descriptor = &base_block[offset..offset + EDID_DETAILED_TIMING_SIZE];

        let pixel_clock = u16::from_le_bytes([descriptor[0], descriptor[1]]);
        if pixel_clock == 0 {
            continue;
        }

        modes.push(parse_detailed_timing(descriptor, index == 0)?);
    }

    // TODO: Parse established timings, standard timings, CEA extension modes,
    // and DisplayID modes.
    Ok(modes)
}

fn parse_detailed_timing(descriptor: &[u8], preferred: bool) -> Result<DrmDisplayMode, DrmError> {
    let clock = u32::from(u16::from_le_bytes([descriptor[0], descriptor[1]]))
        .checked_mul(10)
        .ok_or(DrmError::Invalid)?;

    let hdisplay = u16::try_from(u32::from(descriptor[2]) | (u32::from(descriptor[4] & 0xf0) << 4))
        .map_err(|_| DrmError::Invalid)?;
    let hblank = u16::try_from(u32::from(descriptor[3]) | (u32::from(descriptor[4] & 0x0f) << 8))
        .map_err(|_| DrmError::Invalid)?;
    let vdisplay = u16::try_from(u32::from(descriptor[5]) | (u32::from(descriptor[7] & 0xf0) << 4))
        .map_err(|_| DrmError::Invalid)?;
    let vblank = u16::try_from(u32::from(descriptor[6]) | (u32::from(descriptor[7] & 0x0f) << 8))
        .map_err(|_| DrmError::Invalid)?;

    if hdisplay == 0 || hblank == 0 || vdisplay == 0 || vblank == 0 {
        return Err(DrmError::Invalid);
    }

    let hsync_offset =
        u16::try_from(u32::from(descriptor[8]) | (u32::from(descriptor[11] & 0xc0) << 2))
            .map_err(|_| DrmError::Invalid)?;
    let hsync_width =
        u16::try_from(u32::from(descriptor[9]) | (u32::from(descriptor[11] & 0x30) << 4))
            .map_err(|_| DrmError::Invalid)?;
    let vsync_offset =
        u16::try_from(u32::from(descriptor[10] >> 4) | (u32::from(descriptor[11] & 0x0c) << 2))
            .map_err(|_| DrmError::Invalid)?;
    let vsync_width =
        u16::try_from(u32::from(descriptor[10] & 0x0f) | (u32::from(descriptor[11] & 0x03) << 4))
            .map_err(|_| DrmError::Invalid)?;

    let hsync_start = hdisplay
        .checked_add(hsync_offset)
        .ok_or(DrmError::Invalid)?;
    let hsync_end = hsync_start
        .checked_add(hsync_width)
        .ok_or(DrmError::Invalid)?;
    let htotal = hdisplay.checked_add(hblank).ok_or(DrmError::Invalid)?;

    let vsync_start = vdisplay
        .checked_add(vsync_offset)
        .ok_or(DrmError::Invalid)?;
    let vsync_end = vsync_start
        .checked_add(vsync_width)
        .ok_or(DrmError::Invalid)?;
    let vtotal = vdisplay.checked_add(vblank).ok_or(DrmError::Invalid)?;

    let flags = parse_detailed_timing_flags(descriptor[17]);
    let mut type_ = DrmModeType::DRIVER;
    if preferred {
        type_ |= DrmModeType::PREFERRED;
    }

    Ok(DrmModeModeInfo {
        clock,
        hdisplay,
        hsync_start,
        hsync_end,
        htotal,
        hskew: 0,
        vdisplay,
        vsync_start,
        vsync_end,
        vtotal,
        vscan: 0,
        vrefresh: 0,
        flags: flags.bits(),
        type_: type_.bits(),
        name: display_mode_name(hdisplay, vdisplay),
    }
    .into())
}

fn parse_detailed_timing_flags(value: u8) -> DrmModeFlag {
    let mut flags = DrmModeFlag::empty();

    if value & 0x80 != 0 {
        flags |= DrmModeFlag::INTERLACE;
    }

    // The polarity bits are meaningful for digital separate sync detailed
    // timings. Other sync encodings are kept as no-polarity for now.
    if (value & 0x18) == 0x18 {
        flags |= if value & 0x02 != 0 {
            DrmModeFlag::PHSYNC
        } else {
            DrmModeFlag::NHSYNC
        };
        flags |= if value & 0x04 != 0 {
            DrmModeFlag::PVSYNC
        } else {
            DrmModeFlag::NVSYNC
        };
    }

    flags
}

fn display_mode_name(width: u16, height: u16) -> [u8; DRM_DISPLAY_MODE_NAME_LEN] {
    let mut name = [0u8; DRM_DISPLAY_MODE_NAME_LEN];
    let formatted_name = alloc::format!("{width}x{height}");
    let bytes = formatted_name.as_bytes();
    let len = bytes.len().min(DRM_DISPLAY_MODE_NAME_LEN - 1);
    name[..len].copy_from_slice(&bytes[..len]);
    name
}
