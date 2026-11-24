//! The vault file. A random data key seals the body; Argon2id of the
//! login password wraps the data key. Both with XChaCha20-Poly1305. Labels
//! and attributes are sealed too: nothing on disk is in clear but the
//! header.
//!
//! ```text
//! magic "GH0STKR1" | version 1 | argon2id m t p (u32 le) | salt[16]
//! | wrap nonce[24] | wrapped data key[48] | body nonce[24] | sealed body
//! ```
//!
//! The wrap's AD is the header up to the salt; the body's is the whole
//! header, so no field can be swapped without the tag failing.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use zeroize::{Zeroize, Zeroizing};

use crate::db::Db;
use crate::secure::{Locked, random_array};

const MAGIC: &[u8; 8] = b"GH0STKR1";
const VERSION: u8 = 1;
const WRAP_AD: usize = 8 + 1 + 12 + 16;
const HEADER: usize = WRAP_AD + 24 + 48 + 24;

/// Argon2id costs. Memory in KiB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cost {
    pub m: u32,
    pub t: u32,
    pub p: u32,
}

impl Cost {
    /// Unmeasured on the device: tuned there to about 1 s
    /// (docs/hardware-facts.md).
    pub const DEFAULT: Cost = Cost {
        m: 64 * 1024,
        t: 3,
        p: 1,
    };
    /// Fast enough for tests.
    pub const TEST: Cost = Cost { m: 64, t: 1, p: 1 };

    /// A hostile file must not make us allocate gigabytes or spin.
    fn sane(self) -> bool {
        (8..=256 * 1024).contains(&self.m)
            && (1..=16).contains(&self.t)
            && (1..=4).contains(&self.p)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// The password does not open it.
    Wrong,
    /// Not a vault, or damaged.
    Corrupt,
}

pub struct Vault {
    cost: Cost,
    salt: [u8; 16],
    wrap_nonce: [u8; 24],
    wrapped: [u8; 48],
    key: Locked,
}

fn kek(password: &[u8], salt: &[u8; 16], cost: Cost) -> Result<Zeroizing<[u8; 32]>, Error> {
    let params =
        argon2::Params::new(cost.m, cost.t, cost.p, Some(32)).map_err(|_| Error::Corrupt)?;
    let a = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut out = Zeroizing::new([0u8; 32]);
    a.hash_password_into(password, salt, &mut *out)
        .map_err(|_| Error::Corrupt)?;
    Ok(out)
}

fn cipher(key: &[u8]) -> XChaCha20Poly1305 {
    let mut k = [0u8; 32];
    k.copy_from_slice(key);
    let c = XChaCha20Poly1305::new(&Key::from(k));
    k.zeroize();
    c
}

impl Vault {
    /// A new vault with a fresh data key.
    pub fn create(password: &[u8], cost: Cost) -> Result<Vault, Error> {
        let key = Zeroizing::new(random_array::<32>());
        let mut v = Vault {
            cost,
            salt: [0; 16],
            wrap_nonce: [0; 24],
            wrapped: [0; 48],
            key: Locked::new(&*key),
        };
        v.wrap(password, cost)?;
        Ok(v)
    }

    /// Wraps the data key under a new password, with a new salt.
    pub fn wrap(&mut self, password: &[u8], cost: Cost) -> Result<(), Error> {
        self.cost = cost;
        self.salt = random_array();
        self.wrap_nonce = random_array();
        let kek = kek(password, &self.salt, cost)?;
        let ad = self.header(&[0; 24]);
        let ct = cipher(&*kek)
            .encrypt(
                &XNonce::from(self.wrap_nonce),
                Payload {
                    msg: self.key.bytes(),
                    aad: &ad[..WRAP_AD],
                },
            )
            .map_err(|_| Error::Corrupt)?;
        self.wrapped.copy_from_slice(&ct);
        Ok(())
    }

    fn header(&self, body_nonce: &[u8; 24]) -> [u8; HEADER] {
        let mut h = [0u8; HEADER];
        h[..8].copy_from_slice(MAGIC);
        h[8] = VERSION;
        h[9..13].copy_from_slice(&self.cost.m.to_le_bytes());
        h[13..17].copy_from_slice(&self.cost.t.to_le_bytes());
        h[17..21].copy_from_slice(&self.cost.p.to_le_bytes());
        h[21..37].copy_from_slice(&self.salt);
        h[37..61].copy_from_slice(&self.wrap_nonce);
        h[61..109].copy_from_slice(&self.wrapped);
        h[109..].copy_from_slice(body_nonce);
        h
    }

    /// The whole file for `db`, with a fresh body nonce.
    pub fn seal(&self, db: &Db) -> Vec<u8> {
        let nonce: [u8; 24] = random_array();
        let header = self.header(&nonce);
        let plain = Zeroizing::new(db.encode());
        let mut out = header.to_vec();
        match cipher(self.key.bytes()).encrypt(
            &XNonce::from(nonce),
            Payload {
                msg: &plain,
                aad: &header,
            },
        ) {
            Ok(ct) => out.extend_from_slice(&ct),
            // Only on a length past 256 GiB.
            Err(_) => std::process::abort(),
        }
        out
    }

    /// Opens a file with the password. The tag decides, nothing else.
    pub fn open(file: &[u8], password: &[u8]) -> Result<(Vault, Db), Error> {
        if file.len() < HEADER + 16 || &file[..8] != MAGIC || file[8] != VERSION {
            return Err(Error::Corrupt);
        }
        let word =
            |at: usize| u32::from_le_bytes([file[at], file[at + 1], file[at + 2], file[at + 3]]);
        let cost = Cost {
            m: word(9),
            t: word(13),
            p: word(17),
        };
        if !cost.sane() {
            return Err(Error::Corrupt);
        }
        let mut salt = [0u8; 16];
        salt.copy_from_slice(&file[21..37]);
        let mut wrap_nonce = [0u8; 24];
        wrap_nonce.copy_from_slice(&file[37..61]);
        let mut wrapped = [0u8; 48];
        wrapped.copy_from_slice(&file[61..109]);
        let mut body_nonce = [0u8; 24];
        body_nonce.copy_from_slice(&file[109..HEADER]);
        let kek = kek(password, &salt, cost)?;
        let key = Zeroizing::new(
            cipher(&*kek)
                .decrypt(
                    &XNonce::from(wrap_nonce),
                    Payload {
                        msg: &wrapped,
                        aad: &file[..WRAP_AD],
                    },
                )
                .map_err(|_| Error::Wrong)?,
        );
        let plain = Zeroizing::new(
            cipher(&key)
                .decrypt(
                    &XNonce::from(body_nonce),
                    Payload {
                        msg: &file[HEADER..],
                        aad: &file[..HEADER],
                    },
                )
                .map_err(|_| Error::Corrupt)?,
        );
        let db = Db::decode(&plain).ok_or(Error::Corrupt)?;
        Ok((
            Vault {
                cost,
                salt,
                wrap_nonce,
                wrapped,
                key: Locked::new(&key),
            },
            db,
        ))
    }
}

/// Replaces `path` whole: a temp file, fsync, rename, fsync of the folder.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    // A leftover temp file keeps its mode through a truncate: start fresh.
    let _ = std::fs::remove_file(&tmp);
    let mut f = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path)?;
    if let Some(dir) = path.parent() {
        File::open(dir)?.sync_all()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Collection, Item};

    fn db() -> Db {
        let mut db = Db::default();
        db.collections.push(Collection {
            id: "login".into(),
            label: "Login".into(),
            ..Default::default()
        });
        db.collections[0].items.push(Item {
            id: 1,
            label: "git".into(),
            attrs: vec![("service".into(), "github.com".into())],
            secret: Some(Locked::new(b"hunter2")),
            content_type: "text/plain".into(),
            ..Default::default()
        });
        db
    }

    #[test]
    fn a_vault_round_trips() {
        let v = Vault::create(b"pw", Cost::TEST).unwrap();
        let file = v.seal(&db());
        let (_, back) = Vault::open(&file, b"pw").unwrap();
        assert_eq!(back, db());
        assert!(!file.windows(7).any(|w| w == b"hunter2"));
        assert!(
            !file.windows(10).any(|w| w == b"github.com"),
            "attributes are sealed"
        );
    }

    #[test]
    fn the_wrong_password_is_wrong() {
        let file = Vault::create(b"pw", Cost::TEST).unwrap().seal(&db());
        assert_eq!(Vault::open(&file, b"pW").err(), Some(Error::Wrong));
    }

    #[test]
    fn any_changed_byte_is_refused() {
        let file = Vault::create(b"pw", Cost::TEST).unwrap().seal(&db());
        for i in (0..file.len()).step_by(7) {
            let mut bad = file.clone();
            bad[i] ^= 1;
            assert!(Vault::open(&bad, b"pw").is_err(), "byte {i}");
        }
        assert_eq!(
            Vault::open(&file[..file.len() - 1], b"pw").err(),
            Some(Error::Corrupt)
        );
        assert_eq!(Vault::open(&file[..50], b"pw").err(), Some(Error::Corrupt));
    }

    #[test]
    fn a_new_password_keeps_the_contents() {
        let file = Vault::create(b"old", Cost::TEST).unwrap().seal(&db());
        let (mut v, d) = Vault::open(&file, b"old").unwrap();
        v.wrap(b"new", Cost::TEST).unwrap();
        let file = v.seal(&d);
        assert_eq!(Vault::open(&file, b"old").err(), Some(Error::Wrong));
        assert_eq!(Vault::open(&file, b"new").unwrap().1, db());
    }

    #[test]
    fn hostile_costs_are_refused_before_work() {
        let mut file = Vault::create(b"pw", Cost::TEST).unwrap().seal(&db());
        file[9..13].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(Vault::open(&file, b"pw").err(), Some(Error::Corrupt));
    }

    #[test]
    fn writes_replace_the_file() {
        let dir = std::env::temp_dir().join(format!("gh0st-vault-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("login.vault");
        write_atomic(&p, b"one").unwrap();
        write_atomic(&p, b"two").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"two");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
