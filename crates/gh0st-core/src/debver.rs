//! Debian version ordering, as `dpkg --compare-versions` has it (Debian
//! policy 5.6.12): epoch, then upstream, then revision. `~` sorts before
//! everything, even the end of the string.

use std::cmp::Ordering;

/// Compares two Debian versions.
pub fn cmp(a: &str, b: &str) -> Ordering {
    let (ea, ua, ra) = split(a);
    let (eb, ub, rb) = split(b);
    ea.cmp(&eb)
        .then_with(|| part(ua, ub))
        .then_with(|| part(ra, rb))
}

/// True when `new` is a later version than `old`.
pub fn newer(new: &str, old: &str) -> bool {
    cmp(new, old) == Ordering::Greater
}

fn split(v: &str) -> (u64, &str, &str) {
    let v = v.trim();
    let (epoch, rest) = match v.split_once(':') {
        Some((e, r)) if e.bytes().all(|b| b.is_ascii_digit()) => (e.parse().unwrap_or(0), r),
        _ => (0, v),
    };
    match rest.rsplit_once('-') {
        Some((u, r)) => (epoch, u, r),
        None => (epoch, rest, ""),
    }
}

/// dpkg's `verrevcmp`: runs of non-digits by `order`, then runs of digits
/// by value, in turn.
fn part(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.as_bytes(), b.as_bytes());
    while !a.is_empty() || !b.is_empty() {
        let na = a.iter().take_while(|c| !c.is_ascii_digit()).count();
        let nb = b.iter().take_while(|c| !c.is_ascii_digit()).count();
        let (sa, sb) = (&a[..na], &b[..nb]);
        for i in 0..na.max(nb) {
            let o = order(sa.get(i).copied()).cmp(&order(sb.get(i).copied()));
            if o != Ordering::Equal {
                return o;
            }
        }
        a = &a[na..];
        b = &b[nb..];
        let da = a.iter().take_while(|c| c.is_ascii_digit()).count();
        let db = b.iter().take_while(|c| c.is_ascii_digit()).count();
        let o = digits(&a[..da]).cmp(&digits(&b[..db]));
        if o != Ordering::Equal {
            return o;
        }
        a = &a[da..];
        b = &b[db..];
    }
    Ordering::Equal
}

/// Leading zeros dropped, so a longer run is a larger number.
fn digits(d: &[u8]) -> Number<'_> {
    let start = d.iter().take_while(|&&c| c == b'0').count();
    Number(&d[start..])
}

#[derive(PartialEq, Eq)]
struct Number<'a>(&'a [u8]);

impl Ord for Number<'_> {
    fn cmp(&self, o: &Self) -> Ordering {
        self.0.len().cmp(&o.0.len()).then_with(|| self.0.cmp(o.0))
    }
}

impl PartialOrd for Number<'_> {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// `~` first, then the end, then letters, then everything else.
fn order(c: Option<u8>) -> i32 {
    match c {
        Some(b'~') => -1,
        None => 0,
        Some(c) if c.is_ascii_alphabetic() => c as i32,
        Some(c) => c as i32 + 256,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Ordering::*;

    #[test]
    fn orders_as_dpkg_does() {
        let cases = [
            ("1.0", "1.0", Equal),
            ("1.0", "1.1", Less),
            ("1.10", "1.9", Greater),
            ("1.0~rc1", "1.0", Less),
            ("1.0~~", "1.0~", Less),
            ("1.0", "1.0a", Less),
            ("1.0a", "1.0+", Less),
            ("1:0.1", "2.0", Greater),
            ("2.0-1", "2.0-2", Less),
            ("2.0", "2.0-0", Equal),
            ("0.2.0", "0.10.0", Less),
            ("007", "7", Equal),
            ("3.5.1-1", "3.5.1-1+deb13u1", Less),
            ("1.2-3-4", "1.2-3-5", Less),
            ("6.18.47-1+rpt1", "6.18.60-1+rpt1", Less),
        ];
        for (a, b, want) in cases {
            assert_eq!(cmp(a, b), want, "{a} vs {b}");
            assert_eq!(cmp(b, a), want.reverse(), "{b} vs {a}");
        }
    }

    #[test]
    fn newer_is_strict() {
        assert!(newer("0.2.1", "0.2.0"));
        assert!(!newer("0.2.0", "0.2.0"));
        assert!(!newer("0.1.9", "0.2.0"));
    }
}
