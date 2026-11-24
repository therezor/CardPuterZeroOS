//! Times Argon2id on the device at a few costs: `argon_bench`.
use std::time::Instant;

fn main() {
    for (m, t) in [
        (65536, 3),
        (65536, 2),
        (65536, 1),
        (32768, 3),
        (32768, 2),
        (19456, 2),
    ] {
        let p = argon2::Params::new(m, t, 1, Some(32)).unwrap();
        let a = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, p);
        let mut out = [0u8; 32];
        let t0 = Instant::now();
        a.hash_password_into(b"password", b"0123456789abcdef", &mut out)
            .unwrap();
        println!(
            "m={:>3} MiB t={t}: {:>5} ms",
            m / 1024,
            t0.elapsed().as_millis()
        );
    }
}
