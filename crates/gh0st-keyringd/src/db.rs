//! What the vault holds, and its plain encoding inside the seal. Every
//! read is bounds checked: a damaged body is `None`, never a panic.

use crate::secure::Locked;

const FORMAT: u8 = 1;
/// Bounds on what one body may hold.
const MAX_COUNT: u32 = 100_000;
const MAX_FIELD: u32 = 1 << 20;

#[derive(Debug, Default, PartialEq, Clone)]
pub struct Db {
    /// The next item id; ids are never reused.
    pub next_id: u64,
    pub collections: Vec<Collection>,
    /// Alias → collection id.
    pub aliases: Vec<(String, String)>,
    pub approvals: Vec<Approval>,
}

#[derive(Debug, Default, PartialEq, Clone)]
pub struct Collection {
    /// Its object path element.
    pub id: String,
    pub label: String,
    pub created: u64,
    pub modified: u64,
    pub items: Vec<Item>,
}

#[derive(Debug, Default, PartialEq, Clone)]
pub struct Item {
    pub id: u64,
    pub label: String,
    pub attrs: Vec<(String, String)>,
    /// None while locked.
    pub secret: Option<Locked>,
    pub content_type: String,
    pub created: u64,
    pub modified: u64,
    /// The app that stored it.
    pub creator: String,
}

/// An app may read an item it did not create.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Approval {
    pub app: String,
    pub item: u64,
}

impl Item {
    pub fn attr(&self, k: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.as_str())
    }

    /// Every pair in `want` is one of ours.
    pub fn matches(&self, want: &[(String, String)]) -> bool {
        want.iter().all(|(k, v)| self.attr(k) == Some(v))
    }
}

impl Db {
    pub fn collection(&self, id: &str) -> Option<&Collection> {
        self.collections.iter().find(|c| c.id == id)
    }

    pub fn collection_mut(&mut self, id: &str) -> Option<&mut Collection> {
        self.collections.iter_mut().find(|c| c.id == id)
    }

    pub fn alias(&self, name: &str) -> Option<&str> {
        self.aliases
            .iter()
            .find(|(a, _)| a == name)
            .map(|(_, c)| c.as_str())
    }

    pub fn set_alias(&mut self, name: &str, coll: Option<&str>) {
        self.aliases.retain(|(a, _)| a != name);
        if let Some(c) = coll {
            self.aliases.push((name.into(), c.into()));
        }
    }

    pub fn approved(&self, app: &str, item: u64) -> bool {
        self.approvals
            .iter()
            .any(|a| a.app == app && a.item == item)
    }

    /// Drops every secret, except those `keep` says stay.
    pub fn forget_secrets(&mut self, keep: impl Fn(&Item) -> bool) {
        for c in &mut self.collections {
            for i in &mut c.items {
                if !keep(i) {
                    i.secret = None;
                }
            }
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = W(Vec::new());
        w.0.push(FORMAT);
        w.u64(self.next_id);
        w.u32(self.collections.len() as u32);
        for c in &self.collections {
            w.str(&c.id);
            w.str(&c.label);
            w.u64(c.created);
            w.u64(c.modified);
            w.u32(c.items.len() as u32);
            for i in &c.items {
                w.u64(i.id);
                w.str(&i.label);
                w.u32(i.attrs.len() as u32);
                for (k, v) in &i.attrs {
                    w.str(k);
                    w.str(v);
                }
                w.bytes(i.secret.as_ref().map(Locked::bytes).unwrap_or_default());
                w.str(&i.content_type);
                w.u64(i.created);
                w.u64(i.modified);
                w.str(&i.creator);
            }
        }
        w.u32(self.aliases.len() as u32);
        for (a, c) in &self.aliases {
            w.str(a);
            w.str(c);
        }
        w.u32(self.approvals.len() as u32);
        for a in &self.approvals {
            w.str(&a.app);
            w.u64(a.item);
        }
        w.0
    }

    pub fn decode(b: &[u8]) -> Option<Db> {
        let mut r = R { b, pos: 0 };
        if r.take(1)? != [FORMAT] {
            return None;
        }
        let mut db = Db {
            next_id: r.u64()?,
            ..Db::default()
        };
        for _ in 0..r.count()? {
            let mut c = Collection {
                id: r.str()?,
                label: r.str()?,
                created: r.u64()?,
                modified: r.u64()?,
                items: Vec::new(),
            };
            for _ in 0..r.count()? {
                let id = r.u64()?;
                let label = r.str()?;
                let mut attrs = Vec::new();
                for _ in 0..r.count()? {
                    attrs.push((r.str()?, r.str()?));
                }
                let secret = Some(Locked::new(r.bytes()?));
                c.items.push(Item {
                    id,
                    label,
                    attrs,
                    secret,
                    content_type: r.str()?,
                    created: r.u64()?,
                    modified: r.u64()?,
                    creator: r.str()?,
                });
            }
            db.collections.push(c);
        }
        for _ in 0..r.count()? {
            db.aliases.push((r.str()?, r.str()?));
        }
        for _ in 0..r.count()? {
            db.approvals.push(Approval {
                app: r.str()?,
                item: r.u64()?,
            });
        }
        (r.pos == b.len()).then_some(db)
    }
}

struct W(Vec<u8>);

impl W {
    fn u32(&mut self, n: u32) {
        self.0.extend_from_slice(&n.to_le_bytes());
    }
    fn u64(&mut self, n: u64) {
        self.0.extend_from_slice(&n.to_le_bytes());
    }
    fn bytes(&mut self, b: &[u8]) {
        self.u32(b.len() as u32);
        self.0.extend_from_slice(b);
    }
    fn str(&mut self, s: &str) {
        self.bytes(s.as_bytes());
    }
}

struct R<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> R<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.pos..self.pos.checked_add(n)?)?;
        self.pos += n;
        Some(s)
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn count(&mut self) -> Option<u32> {
        self.u32().filter(|&n| n <= MAX_COUNT)
    }
    fn bytes(&mut self) -> Option<&'a [u8]> {
        let n = self.u32().filter(|&n| n <= MAX_FIELD)?;
        self.take(n as usize)
    }
    fn str(&mut self) -> Option<String> {
        String::from_utf8(self.bytes()?.to_vec()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Db {
        Db {
            next_id: 3,
            collections: vec![Collection {
                id: "login".into(),
                label: "Login".into(),
                created: 1,
                modified: 2,
                items: vec![Item {
                    id: 2,
                    label: "Wi-Fi Home".into(),
                    attrs: vec![("setting-key".into(), "psk".into())],
                    secret: Some(Locked::new(b"s3cret")),
                    content_type: "text/plain".into(),
                    created: 5,
                    modified: 6,
                    creator: "nm".into(),
                }],
            }],
            aliases: vec![("default".into(), "login".into())],
            approvals: vec![Approval {
                app: "chromium".into(),
                item: 2,
            }],
        }
    }

    #[test]
    fn round_trips() {
        assert_eq!(Db::decode(&sample().encode()), Some(sample()));
        assert_eq!(Db::decode(&Db::default().encode()), Some(Db::default()));
    }

    #[test]
    fn damaged_bodies_are_none() {
        let good = sample().encode();
        for n in 0..good.len() {
            assert_eq!(Db::decode(&good[..n]), None);
        }
        let mut long = good.clone();
        long.push(0);
        assert_eq!(Db::decode(&long), None);
        // Mutations: never a panic.
        let mut x = 0x1234_5678_u64;
        for _ in 0..20_000 {
            let mut m = good.clone();
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let i = (x as usize) % m.len();
            m[i] = (x >> 32) as u8;
            let _ = Db::decode(&m);
        }
    }

    #[test]
    fn lookups() {
        let mut db = sample();
        assert_eq!(db.alias("default"), Some("login"));
        db.set_alias("default", None);
        assert_eq!(db.alias("default"), None);
        assert!(db.approved("chromium", 2) && !db.approved("chromium", 1));
        let item = &db.collections[0].items[0];
        assert!(item.matches(&[("setting-key".into(), "psk".into())]));
        assert!(!item.matches(&[("setting-key".into(), "x".into())]));
        db.forget_secrets(|_| false);
        assert!(db.collections[0].items[0].secret.is_none());
    }
}
