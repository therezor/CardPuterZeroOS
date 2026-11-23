//! App icons from the pack, decoded only for rows near the selection and
//! kept in a small cache: a 20 px icon is about 1 KB.

use gh0st_app::gfx::image::Image;
use gh0st_core::store::Icons;
use std::cell::RefCell;
use std::rc::Rc;

const KEEP: usize = 24;

/// Package, size, picture (None: no icon or a bad one).
type Entry = (String, u32, Option<Rc<Image>>);

#[derive(Default)]
pub struct Pics {
    /// Most recent last.
    cache: RefCell<Vec<Entry>>,
}

impl Pics {
    pub fn get(&self, icons: Option<&Icons>, package: &str, size: u32) -> Option<Rc<Image>> {
        let mut c = self.cache.borrow_mut();
        if let Some(i) = c.iter().position(|(p, s, _)| p == package && *s == size) {
            let hit = c.remove(i);
            let img = hit.2.clone();
            c.push(hit);
            return img;
        }
        let img = icons
            .and_then(|i| i.png(package))
            .and_then(|png| Image::from_png(png, size).ok())
            .map(Rc::new);
        if c.len() >= KEEP {
            c.remove(0);
        }
        c.push((package.to_string(), size, img.clone()));
        img
    }

    /// After a new pack lands.
    pub fn clear(&self) {
        self.cache.borrow_mut().clear();
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.cache.borrow().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_a_few_and_remembers_misses() {
        let p = Pics::default();
        for i in 0..40 {
            assert!(p.get(None, &format!("app{i}"), 20).is_none());
        }
        assert_eq!(p.len(), KEEP);
    }
}
