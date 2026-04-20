// SPDX-License-Identifier: MPL-2.0

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RectU32 {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

impl RectU32 {
    pub fn x(&self) -> u32 {
        self.x
    }

    pub fn y(&self) -> u32 {
        self.y
    }

    pub fn width(&self) -> u32 {
        self.w
    }

    pub fn height(&self) -> u32 {
        self.h
    }
}
