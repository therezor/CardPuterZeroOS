//! Pixels for every gh0st program: an RGB565 canvas, baked fonts and icons,
//! the shared widgets, and the ways a frame leaves the process.

pub mod canvas;
pub mod color;
pub mod display;
pub mod drm;
pub mod fb;
pub mod font;
pub mod geom;
pub mod icon;
pub mod image;
pub mod lint;
pub mod qr;
pub mod snapshot;
pub mod theme;
pub mod wallpaper;
pub mod widgets;

pub use canvas::Canvas;
pub use color::Color;
pub use geom::Rect;
pub use theme::Theme;
