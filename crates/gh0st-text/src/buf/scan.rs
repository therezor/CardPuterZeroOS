//! Byte search eight bytes at a time. No crate: these three are all the
//! editor needs, and they run over every byte of a file once while it opens.

const LO: u64 = 0x0101_0101_0101_0101;
const HI: u64 = 0x8080_8080_8080_8080;

/// A word with the high bit set in each byte of `w` that is zero.
#[inline]
fn zeros(w: u64) -> u64 {
    w.wrapping_sub(LO) & !w & HI
}

#[inline]
fn word(h: &[u8]) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&h[..8]);
    u64::from_le_bytes(b)
}

/// The first `b` in `h`.
pub fn memchr(b: u8, h: &[u8]) -> Option<usize> {
    let pat = LO.wrapping_mul(b as u64);
    let mut i = 0;
    while i + 8 <= h.len() {
        let z = zeros(word(&h[i..]) ^ pat);
        if z != 0 {
            return Some(i + (z.trailing_zeros() / 8) as usize);
        }
        i += 8;
    }
    h[i..].iter().position(|&c| c == b).map(|p| i + p)
}

/// The last `b` in `h`.
pub fn memrchr(b: u8, h: &[u8]) -> Option<usize> {
    let pat = LO.wrapping_mul(b as u64);
    let mut end = h.len();
    while end >= 8 {
        let z = zeros(word(&h[end - 8..]) ^ pat);
        if z != 0 {
            // The highest flagged byte; a borrow can only flag bytes above a
            // true zero, never below, so the top flag may be false: check.
            let mut j = 8 - (z.leading_zeros() / 8) as usize;
            while j > 0 {
                if h[end - 8 + j - 1] == b {
                    return Some(end - 8 + j - 1);
                }
                j -= 1;
            }
        }
        end -= 8;
    }
    h[..end].iter().rposition(|&c| c == b)
}

/// How many `b` there are in `h`.
pub fn count(b: u8, h: &[u8]) -> usize {
    // The compiler turns this into vector compares.
    h.iter().filter(|&&c| c == b).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slow_chr(b: u8, h: &[u8]) -> Option<usize> {
        h.iter().position(|&c| c == b)
    }

    fn slow_rchr(b: u8, h: &[u8]) -> Option<usize> {
        h.iter().rposition(|&c| c == b)
    }

    #[test]
    fn agrees_with_the_slow_search() {
        let mut seed: u32 = 7;
        for len in 0..80 {
            for _ in 0..50 {
                let h: Vec<u8> = (0..len)
                    .map(|_| {
                        seed ^= seed << 13;
                        seed ^= seed >> 17;
                        seed ^= seed << 5;
                        // Few distinct bytes, so hits and near misses are common.
                        [b'\n', b'a', 0, 0xff, 9, 11][seed as usize % 6]
                    })
                    .collect();
                for b in [b'\n', 0, 0xff, b'z', 10, 11] {
                    assert_eq!(memchr(b, &h), slow_chr(b, &h), "{b} in {h:?}");
                    assert_eq!(memrchr(b, &h), slow_rchr(b, &h), "{b} in {h:?}");
                    assert_eq!(count(b, &h), h.iter().filter(|&&c| c == b).count());
                }
            }
        }
    }
}
