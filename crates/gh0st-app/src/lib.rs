#![doc = include_str!("../../../docs/sdk.md")]

mod bars;
mod field;
mod key;
mod list;
mod page;
mod run;
mod runtime;
pub mod system;
pub mod testing;
mod ui;

pub use bars::{Bars, SoftKey};
pub use field::{Edit, TextField};
/// A key event as [`Page::raw`] gets it.
pub use gh0st_core::keys::{Key as RawKey, State as KeyState};
pub use key::{Key, codes};
pub use list::{Action, List, Row};
pub use page::{Cx, Job, JobEvent, Page, Settings, Timer, Watch};
pub use run::run;
pub use ui::Ui;

/// Pixels, for pages that draw their own: the canvas, colours, rects, the
/// theme, fonts and icons.
pub use gh0st_ui as gfx;

/// Everything a page needs: `use gh0st_app::prelude::*;`.
pub mod prelude {
    pub use crate::{
        Action, Bars, Cx, Edit, Job, JobEvent, Key, List, Page, Row, SoftKey, TextField, Timer, Ui,
        Watch,
    };
}
