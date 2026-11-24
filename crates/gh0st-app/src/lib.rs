// Release builds abort on a panic, and here a panic takes the screen with it.
#![cfg_attr(not(test), deny(clippy::unwrap_used))]
#![doc = include_str!("../../../docs/sdk.md")]

mod bars;
mod command;
mod dialog;
mod field;
mod form;
pub mod frames;
pub mod helper;
mod key;
mod list;
pub mod media;
mod menu;
mod page;
pub mod portal;
mod run;
mod runtime;
mod search;
pub mod secret;
pub mod sidecar;
mod steps;
pub mod system;
mod tiles;
/// Notices in the shell's notifications: the file format and folders, for
/// posting with more than [`Cx::post`] gives (a count, a tone, another app
/// to open).
pub use gh0st_core::notify;
/// Time, the clock and poll, as the SDK's loop uses them.
pub use gh0st_core::rt;
pub mod testing;
mod ui;
pub mod watch;

pub use bars::{Bars, Chip};
pub use command::{Command, Commands, Hit, Step};
pub use dialog::{Answer, Dialog};
pub use field::{Edit, TextField};
pub use form::Form;
/// A key event as [`Page::raw`] gets it.
pub use gh0st_core::keys::{Key as RawKey, State as KeyState};
/// System sounds, for [`Cx::sound`].
pub use gh0st_core::sound::Sound;
/// What a toast or a top-bar word says about itself.
pub use gh0st_ui::theme::Tone;
pub use key::{Key, codes};
pub use list::{Action, Grid, List, Row, Scroll, cycle};
pub use page::{Cx, Handler, Job, JobEvent, Page, Pick, Settings, Timer, Watch};
pub use portal::{PickMode, PickSpec};
pub use run::{Options, run, run_with};
pub use search::Search;
pub use steps::Steps;
pub use tiles::Tiles;
pub use ui::Ui;

/// Pixels, for pages that draw their own: the canvas, colours, rects, the
/// theme, fonts and icons.
pub use gh0st_ui as gfx;

/// Everything a page needs: `use gh0st_app::prelude::*;`.
pub mod prelude {
    pub use crate::{
        Action, Answer, Bars, Chip, Command, Commands, Cx, Dialog, Edit, Form, Grid, Job, JobEvent,
        Key, List, Page, Row, Scroll, Search, Sound, Step, Steps, TextField, Timer, Tone, Ui,
        Watch,
    };
}
