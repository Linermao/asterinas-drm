// SPDX-License-Identifier: MPL-2.0

use core::ops::RangeInclusive;

use aster_util::fixed_point::FixedU64;

/// A two-dimensional size in units specified by `T`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DrmSize<T = u32> {
    width: T,
    height: T,
}

impl<T: Copy> DrmSize<T> {
    pub fn new(width: T, height: T) -> Self {
        Self { width, height }
    }

    pub fn width(&self) -> T {
        self.width
    }

    pub fn height(&self) -> T {
        self.height
    }
}

impl DrmSize<u32> {
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

impl DrmSize<DrmU16F16> {
    pub fn is_empty(&self) -> bool {
        self.width.raw() == 0 || self.height.raw() == 0
    }
}

impl<T: PartialOrd> DrmSize<T> {
    /// Returns whether the size is within the given inclusive limits.
    pub fn is_within(
        &self,
        width_range: RangeInclusive<T>,
        height_range: RangeInclusive<T>,
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
pub struct DrmRect<Coord = u32, Extent = Coord> {
    x: Coord,
    y: Coord,
    size: DrmSize<Extent>,
}

impl<Coord: Copy, Extent: Copy> DrmRect<Coord, Extent> {
    pub fn new(x: Coord, y: Coord, width: Extent, height: Extent) -> Self {
        Self {
            x,
            y,
            size: DrmSize::new(width, height),
        }
    }

    pub fn x(&self) -> Coord {
        self.x
    }

    pub fn y(&self) -> Coord {
        self.y
    }

    pub fn size(&self) -> DrmSize<Extent> {
        self.size
    }

    pub fn width(&self) -> Extent {
        self.size.width()
    }

    pub fn height(&self) -> Extent {
        self.size.height()
    }

    pub fn set_x(&mut self, x: Coord) {
        self.x = x;
    }

    pub fn set_y(&mut self, y: Coord) {
        self.y = y;
    }

    pub fn set_width(&mut self, width: Extent) {
        self.size.width = width;
    }

    pub fn set_height(&mut self, height: Extent) {
        self.size.height = height;
    }
}

impl<Coord> DrmRect<Coord, u32> {
    pub fn is_empty(&self) -> bool {
        self.size.is_empty()
    }
}

impl DrmRect<u32> {
    /// Returns the exclusive right edge, widened to hold the sum of two `u32` values.
    pub fn right(&self) -> u64 {
        u64::from(self.x) + u64::from(self.width())
    }

    /// Returns the exclusive bottom edge, widened to hold the sum of two `u32` values.
    pub fn bottom(&self) -> u64 {
        u64::from(self.y) + u64::from(self.height())
    }

    /// Returns whether the point is inside the rectangle.
    ///
    /// The left/top edges are inclusive and the right/bottom edges are exclusive.
    pub fn contains_point(&self, x: u32, y: u32) -> bool {
        self.x <= x && x - self.x < self.width() && self.y <= y && y - self.y < self.height()
    }

    /// Returns whether `other` is fully contained within `self`.
    pub fn contains_rect(&self, other: &Self) -> bool {
        self.x <= other.x
            && other.right() <= self.right()
            && self.y <= other.y
            && other.bottom() <= self.bottom()
    }
}

/// A plane's destination rectangle in integer CRTC pixels.
pub type DrmCrtcRect = DrmRect<i32, u32>;

impl DrmCrtcRect {
    /// Returns the exclusive right edge, widened to hold a signed coordinate plus a width.
    pub fn right(&self) -> i64 {
        i64::from(self.x) + i64::from(self.width())
    }

    /// Returns the exclusive bottom edge, widened to hold a signed coordinate plus a height.
    pub fn bottom(&self) -> i64 {
        i64::from(self.y) + i64::from(self.height())
    }

    /// Returns whether the point is inside the rectangle.
    ///
    /// The left/top edges are inclusive and the right/bottom edges are exclusive.
    pub fn contains_point(&self, x: i32, y: i32) -> bool {
        self.x <= x && i64::from(x) < self.right() && self.y <= y && i64::from(y) < self.bottom()
    }

    /// Returns whether `other` is fully contained within `self`.
    pub fn contains_rect(&self, other: &Self) -> bool {
        self.x <= other.x
            && other.right() <= self.right()
            && self.y <= other.y
            && other.bottom() <= self.bottom()
    }
}

/// A plane's source rectangle in unsigned 16.16 framebuffer coordinates.
pub type DrmSourceRect = DrmRect<DrmU16F16>;

impl DrmSourceRect {
    pub fn is_empty(&self) -> bool {
        self.size.is_empty()
    }

    /// Returns the exclusive right edge in unsigned 48.16 framebuffer coordinates.
    pub fn right(&self) -> FixedU64<16> {
        FixedU64::from_raw(self.raw_rect().right())
    }

    /// Returns the exclusive bottom edge in unsigned 48.16 framebuffer coordinates.
    pub fn bottom(&self) -> FixedU64<16> {
        FixedU64::from_raw(self.raw_rect().bottom())
    }

    /// Returns whether the point is inside the source rectangle.
    ///
    /// The left/top edges are inclusive and the right/bottom edges are exclusive.
    pub fn contains_point(&self, x: DrmU16F16, y: DrmU16F16) -> bool {
        self.raw_rect().contains_point(x.raw(), y.raw())
    }

    /// Returns whether `other` is fully contained within the source rectangle.
    pub fn contains_rect(&self, other: &Self) -> bool {
        self.raw_rect().contains_rect(&other.raw_rect())
    }

    /// Converts integer pixels to 16.16, returning `None` if any field is too large.
    pub fn try_from_pixel_rect(rect: DrmRect<u32>) -> Option<Self> {
        Some(Self::new(
            DrmU16F16::try_from_pixels(rect.x())?,
            DrmU16F16::try_from_pixels(rect.y())?,
            DrmU16F16::try_from_pixels(rect.width())?,
            DrmU16F16::try_from_pixels(rect.height())?,
        ))
    }

    /// Converts to integer pixels, returning `None` if any field has a fractional part.
    pub fn try_to_pixel_rect(&self) -> Option<DrmRect<u32>> {
        Some(DrmRect::new(
            self.x().try_to_pixels()?,
            self.y().try_to_pixels()?,
            self.width().try_to_pixels()?,
            self.height().try_to_pixels()?,
        ))
    }

    /// Returns whether the source rectangle lies entirely within the framebuffer.
    pub fn is_within_framebuffer(&self, size: DrmSize) -> bool {
        self.right().raw() <= u64::from(size.width()) << DrmU16F16::FRAC_BITS
            && self.bottom().raw() <= u64::from(size.height()) << DrmU16F16::FRAC_BITS
    }

    // All fields use raw 16.16 units, so integer geometry preserves fractional precision.
    fn raw_rect(&self) -> DrmRect<u32> {
        DrmRect::new(
            self.x().raw(),
            self.y().raw(),
            self.width().raw(),
            self.height().raw(),
        )
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

    /// Converts integer pixels to 16.16, returning `None` on overflow.
    pub const fn try_from_pixels(pixels: u32) -> Option<Self> {
        if pixels > u32::MAX >> Self::FRAC_BITS {
            return None;
        }
        Some(Self::from_raw(pixels << Self::FRAC_BITS))
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
