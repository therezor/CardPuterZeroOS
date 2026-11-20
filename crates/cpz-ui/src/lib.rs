//! Pixels for every cpz program: an RGB565 canvas, baked fonts and icons,
//! the shared widgets, and the ways a frame leaves the process.

pub mod canvas;
pub mod color;
pub mod font;
pub mod geom;
pub mod icon;
pub mod image;
pub mod snapshot;
pub mod theme;
pub mod widgets;

pub use canvas::Canvas;
pub use color::Color;
pub use geom::Rect;
pub use theme::Theme;
