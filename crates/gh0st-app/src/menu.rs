//! The app menu. Fn+H (Help) opens it in every app; the SDK draws it from
//! the items the page in front lists, and adds Close app last. The rules
//! are in the guide, "The app menu".

use crate::list::List;

/// One entry in the app menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem {
    pub(crate) id: u32,
    pub(crate) label: String,
    pub(crate) key: Option<char>,
    pub(crate) value: Option<String>,
}

impl MenuItem {
    /// `id` comes back to [`Page::menu_pick`](crate::Page::menu_pick).
    pub fn new(id: u32, label: impl Into<String>) -> MenuItem {
        MenuItem {
            id,
            label: label.into(),
            key: None,
            value: None,
        }
    }

    /// A letter or digit that picks the item at once while the menu is
    /// open. Never a symbol: those need the Sym key.
    pub fn key(mut self, c: char) -> MenuItem {
        self.key = Some(c.to_ascii_lowercase());
        self
    }

    /// The current state, on the right: "Magnify", "On", "8x14".
    pub fn value(mut self, v: impl Into<String>) -> MenuItem {
        self.value = Some(v.into());
        self
    }
}

/// The id of the item the SDK adds to every menu.
pub(crate) const CLOSE_APP: u32 = u32::MAX;

pub(crate) struct Menu {
    /// The page that listed the items.
    pub owner: usize,
    pub items: Vec<MenuItem>,
    pub list: List,
}

impl Menu {
    pub fn new(owner: usize, mut items: Vec<MenuItem>) -> Menu {
        items.push(MenuItem::new(CLOSE_APP, "Close app"));
        Menu {
            owner,
            items,
            list: List::new(),
        }
    }

    /// The right-hand text of each row: the value, then the key.
    pub fn trails(&self) -> Vec<String> {
        self.items
            .iter()
            .map(|i| match (&i.value, i.key) {
                (Some(v), Some(k)) => format!("{v}   {k}"),
                (Some(v), None) => v.clone(),
                (None, Some(k)) => k.to_string(),
                (None, None) => String::new(),
            })
            .collect()
    }
}
