//! QR codes drawn as square modules, as large as the space allows.

use crate::theme::{QR_DARK, QR_LIGHT, QR_QUIET};
use crate::{Canvas, Rect};
use qrcodegen::{QrCode, QrCodeEcc};

pub struct Qr(QrCode);

impl Qr {
    /// Low error correction: a screen is never smudged, and the lower
    /// version gives bigger modules. Raised when that costs no size.
    pub fn new(text: &str) -> Option<Qr> {
        QrCode::encode_text(text, QrCodeEcc::Low).ok().map(Qr)
    }

    /// Modules on a side, without the quiet zone.
    pub fn modules(&self) -> i32 {
        self.0.size()
    }

    /// The whole-pixel module size that fits `side`, and the drawn side.
    pub fn fit(&self, side: i32) -> (i32, i32) {
        let n = self.modules() + 2 * QR_QUIET;
        let scale = (side / n).max(1);
        (scale, scale * n)
    }

    /// Draws the code with its quiet zone in a square at (x, y) no larger
    /// than `side`. Returns the drawn side.
    pub fn draw(&self, c: &mut Canvas, x: i32, y: i32, side: i32) -> i32 {
        let (scale, drawn) = self.fit(side);
        c.fill(Rect::new(x, y, drawn, drawn), QR_LIGHT);
        let origin = QR_QUIET * scale;
        for my in 0..self.modules() {
            for mx in 0..self.modules() {
                if self.0.get_module(mx, my) {
                    c.fill(
                        Rect::new(
                            x + origin + mx * scale,
                            y + origin + my * scale,
                            scale,
                            scale,
                        ),
                        QR_DARK,
                    );
                }
            }
        }
        drawn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_longest_hotspot_still_fits_at_two_pixels() {
        let worst = format!("WIFI:T:SAE;S:{};P:{};;", r"\;".repeat(16), r"\;".repeat(31));
        let qr = Qr::new(&worst).unwrap();
        let (scale, drawn) = qr.fit(122);
        assert!(scale >= 2, "{} modules", qr.modules());
        assert!(drawn <= 122);
        let usual = Qr::new("WIFI:T:SAE;S:zero-abcd;P:kfwmtqzrapdh;;").unwrap();
        assert!(usual.fit(122).0 >= 3, "{} modules", usual.modules());
    }

    #[test]
    fn modules_are_dark_on_light() {
        let qr = Qr::new("hello").unwrap();
        let mut c = Canvas::new(80, 80);
        let side = qr.draw(&mut c, 0, 0, 80);
        let (scale, _) = qr.fit(80);
        assert_eq!(c.get(0, 0), QR_LIGHT, "quiet zone");
        let finder = QR_QUIET * scale;
        assert_eq!(c.get(finder, finder), QR_DARK, "finder corner");
        assert!(side <= 80);
    }
}
