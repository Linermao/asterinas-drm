// SPDX-License-Identifier: MPL-2.0

use core::ops::RangeInclusive;

use aster_core::prelude::*;

/// A two-dimensional size in pixels.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DrmSize {
    width: u32,
    height: u32,
}

impl DrmSize {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Returns whether the size is within the given inclusive limits.
    pub fn is_within(
        &self,
        width_range: RangeInclusive<u32>,
        height_range: RangeInclusive<u32>,
    ) -> bool {
        width_range.contains(&self.width) && height_range.contains(&self.height)
    }
}

/// Rectangles are checked by their right/bottom edges:
///
/// ```text
/// (x, y)        width        right = x + width
///    +-------------------------+
///    |                         |
///    |                         | height
///    |                         |
///    +-------------------------+
///                            bottom = y + height
/// ```
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DrmRect {
    x: u32,
    y: u32,
    size: DrmSize,
}

impl DrmRect {
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            size: DrmSize::new(width, height),
        }
    }

    pub fn x(&self) -> u32 {
        self.x
    }

    pub fn y(&self) -> u32 {
        self.y
    }

    pub fn size(&self) -> DrmSize {
        self.size
    }

    pub fn width(&self) -> u32 {
        self.size.width()
    }

    pub fn height(&self) -> u32 {
        self.size.height()
    }

    pub fn is_empty(&self) -> bool {
        self.size.is_empty()
    }

    pub fn right(&self) -> Option<u32> {
        self.x.checked_add(self.size.width())
    }

    pub fn bottom(&self) -> Option<u32> {
        self.y.checked_add(self.size.height())
    }

    /// Returns whether the point is inside the rectangle.
    ///
    /// The left/top edges are inclusive and the right/bottom edges are exclusive.
    pub fn contains_point(&self, x: u32, y: u32) -> bool {
        let Some(right) = self.right() else {
            return false;
        };
        let Some(bottom) = self.bottom() else {
            return false;
        };

        (self.x..right).contains(&x) && (self.y..bottom).contains(&y)
    }

    /// Returns whether `other` is fully contained within `self`.
    pub fn contains_rect(&self, other: &Self) -> bool {
        let Some(self_right) = self.right() else {
            return false;
        };
        let Some(self_bottom) = self.bottom() else {
            return false;
        };
        let Some(other_right) = other.right() else {
            return false;
        };
        let Some(other_bottom) = other.bottom() else {
            return false;
        };

        self.x <= other.x
            && other_right <= self_right
            && self.y <= other.y
            && other_bottom <= self_bottom
    }

    pub fn set_x(&mut self, x: u32) {
        self.x = x;
    }

    pub fn set_y(&mut self, y: u32) {
        self.y = y;
    }

    pub fn set_width(&mut self, width: u32) {
        self.size.width = width;
    }

    pub fn set_height(&mut self, height: u32) {
        self.size.height = height;
    }
}

/// A plane's destination rectangle in integer CRTC pixels.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DrmCrtcRect {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

impl DrmCrtcRect {
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn x(&self) -> i32 {
        self.x
    }

    pub fn y(&self) -> i32 {
        self.y
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn set_x(&mut self, x: i32) {
        self.x = x;
    }

    pub fn set_y(&mut self, y: i32) {
        self.y = y;
    }

    pub fn set_width(&mut self, width: u32) {
        self.width = width;
    }

    pub fn set_height(&mut self, height: u32) {
        self.height = height;
    }

    /// Returns whether `other` is fully contained within `self`.
    pub fn contains_rect(&self, other: &Self) -> bool {
        let right = i64::from(self.x) + i64::from(self.width);
        let bottom = i64::from(self.y) + i64::from(self.height);
        let other_right = i64::from(other.x) + i64::from(other.width);
        let other_bottom = i64::from(other.y) + i64::from(other.height);

        self.x <= other.x && self.y <= other.y && other_right <= right && other_bottom <= bottom
    }
}

/// A plane's source rectangle in unsigned 16.16 framebuffer coordinates.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DrmSourceRect {
    x: DrmU16F16,
    y: DrmU16F16,
    width: DrmU16F16,
    height: DrmU16F16,
}

impl DrmSourceRect {
    pub fn new(x: DrmU16F16, y: DrmU16F16, width: DrmU16F16, height: DrmU16F16) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn x(&self) -> DrmU16F16 {
        self.x
    }

    pub fn y(&self) -> DrmU16F16 {
        self.y
    }

    pub fn width(&self) -> DrmU16F16 {
        self.width
    }

    pub fn height(&self) -> DrmU16F16 {
        self.height
    }

    pub fn set_x(&mut self, x: DrmU16F16) {
        self.x = x;
    }

    pub fn set_y(&mut self, y: DrmU16F16) {
        self.y = y;
    }

    pub fn set_width(&mut self, width: DrmU16F16) {
        self.width = width;
    }

    pub fn set_height(&mut self, height: DrmU16F16) {
        self.height = height;
    }

    /// Returns whether the source rectangle lies entirely within an integer pixel rectangle.
    pub fn is_within_pixel_rect(&self, rect: DrmRect) -> bool {
        let left = u64::from(self.x.raw());
        let top = u64::from(self.y.raw());
        let right = left + u64::from(self.width.raw());
        let bottom = top + u64::from(self.height.raw());

        let rect_left = u64::from(rect.x()) << DrmU16F16::FRAC_BITS;
        let rect_top = u64::from(rect.y()) << DrmU16F16::FRAC_BITS;
        let rect_right = rect_left + (u64::from(rect.width()) << DrmU16F16::FRAC_BITS);
        let rect_bottom = rect_top + (u64::from(rect.height()) << DrmU16F16::FRAC_BITS);

        rect_left <= left && rect_top <= top && right <= rect_right && bottom <= rect_bottom
    }
}

/// An unsigned 16.16 source coordinate or extent in framebuffer pixels.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct DrmU16F16(u32);

impl DrmU16F16 {
    const FRAC_BITS: u32 = 16;
    const FRAC_MASK: u32 = (1 << Self::FRAC_BITS) - 1;

    /// Creates a source value from its 16.16 representation.
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    /// Returns the 16.16 representation.
    pub const fn raw(self) -> u32 {
        self.0
    }

    /// Converts integer pixels to 16.16, returning `EINVAL` on overflow.
    pub fn try_from_pixels(pixels: u32) -> Result<Self> {
        if pixels > u32::MAX >> Self::FRAC_BITS {
            return Err(errno!(EINVAL, "the pixel value exceeds the 16.16 range"));
        }
        Ok(Self::from_raw(pixels << Self::FRAC_BITS))
    }

    /// Returns the integer part in pixels, discarding the fractional part.
    pub const fn integer_part(self) -> u32 {
        self.raw() >> Self::FRAC_BITS
    }

    /// Returns the exact integer pixel value, or `None` if there is a fractional part.
    pub const fn try_to_pixels(self) -> Option<u32> {
        if self.raw() & Self::FRAC_MASK != 0 {
            return None;
        }
        Some(self.integer_part())
    }
}
