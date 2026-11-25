//! Plain geometry and color types used in the public API.

use windows::Win32::Foundation::{COLORREF, RECT};

/// A rectangle in client coordinates, in pixels.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    #[must_use]
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }
}

/// A width and height in pixels.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Size {
    pub width: i32,
    pub height: i32,
}

impl From<RECT> for Size {
    fn from(r: RECT) -> Size {
        Size {
            width: r.right - r.left,
            height: r.bottom - r.top,
        }
    }
}

/// An opaque RGB color.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    #[must_use]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b }
    }

    /// Parses `#rrggbb`.
    #[must_use]
    pub fn from_hex(s: &str) -> Option<Color> {
        let hex = s.strip_prefix('#').filter(|h| h.len() == 6)?;
        let [_, r, g, b] = u32::from_str_radix(hex, 16).ok()?.to_be_bytes();
        Some(Color { r, g, b })
    }

    /// The color as a COLORREF (0x00BBGGRR).
    pub(crate) fn colorref(self) -> COLORREF {
        COLORREF(u32::from_le_bytes([self.r, self.g, self.b, 0]))
    }
}

/// A monitor scale as dots per inch; 96 is 100 %.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Dpi(pub u32);

impl Dpi {
    /// Scales a length given for 96 DPI to this DPI.
    #[must_use]
    pub fn scale(self, px: i32) -> i32 {
        rescale(px, 96, self.0)
    }
}

/// `value * to / from`, rounded towards zero and saturated to `i32`.
pub(crate) fn rescale(value: i32, from: u32, to: u32) -> i32 {
    let scaled = i64::from(value) * i64::from(to) / i64::from(from.max(1));
    i32::try_from(scaled).unwrap_or(if scaled < 0 { i32::MIN } else { i32::MAX })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_component_order() {
        let c = Color::from_hex("#1a5fb4").unwrap();
        assert_eq!(c, Color::rgb(0x1a, 0x5f, 0xb4));
        assert_eq!(c.colorref().0, 0x00b4_5f1a);
        assert_eq!(Color::from_hex("blue"), None);
        assert_eq!(Color::from_hex("#12345"), None);
        assert_eq!(Color::from_hex("#gggggg"), None);
    }

    #[test]
    fn scaling() {
        assert_eq!(Dpi(96).scale(26), 26);
        assert_eq!(Dpi(144).scale(26), 39);
        assert_eq!(Dpi(120).scale(-4), -5);
        assert_eq!(rescale(360, 96, 144), 540);
        assert_eq!(rescale(i32::MAX, 96, 192), i32::MAX);
    }
}
