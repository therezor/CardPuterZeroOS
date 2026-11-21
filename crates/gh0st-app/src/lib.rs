#![doc = include_str!("../../../docs/sdk.md")]

mod bars;
mod command;
mod dialog;
mod field;
mod key;
mod list;
mod menu;
mod page;
mod run;
mod runtime;
pub mod system;
pub mod testing;
mod ui;

pub use bars::{Bars, Chip};
pub use command::{Command, Commands, Hit, Step};
pub use dialog::{Answer, Dialog};
pub use field::{Edit, TextField};
/// A key event as [`Page::raw`] gets it.
pub use gh0st_core::keys::{Key as RawKey, State as KeyState};
/// What a toast or a top-bar word says about itself.
pub use gh0st_ui::theme::Tone;
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
        Action, Answer, Bars, Chip, Command, Commands, Cx, Dialog, Edit, Job, JobEvent, Key, List,
        Page, Row, Step, TextField, Timer, Tone, Ui, Watch,
    };
}
