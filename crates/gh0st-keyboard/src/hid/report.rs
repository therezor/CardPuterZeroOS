//! The three reports, as bytes on the wire (no report ID: Bluetooth names
//! it in the characteristic, USB has one kind per function).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Report {
    /// Modifiers, a reserved byte, then up to six keys.
    Keyboard([u8; 8]),
    /// One consumer usage held, or 0 for none.
    Consumer(u16),
    Mouse(Mouse),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mouse {
    /// Bit 0 left, 1 right, 2 middle, 3 back, 4 forward.
    pub buttons: u8,
    pub dx: i8,
    pub dy: i8,
    pub wheel: i8,
    pub pan: i8,
}

pub const KEYBOARD_LEN: usize = 8;
pub const CONSUMER_LEN: usize = 2;
pub const MOUSE_LEN: usize = 5;

impl Report {
    /// The bytes to send, in `buf`; returns how many.
    pub fn bytes(&self, buf: &mut [u8; 8]) -> usize {
        match *self {
            Report::Keyboard(k) => {
                *buf = k;
                KEYBOARD_LEN
            }
            Report::Consumer(u) => {
                buf[..2].copy_from_slice(&u.to_le_bytes());
                CONSUMER_LEN
            }
            Report::Mouse(m) => {
                buf[..5].copy_from_slice(&[
                    m.buttons,
                    m.dx as u8,
                    m.dy as u8,
                    m.wheel as u8,
                    m.pan as u8,
                ]);
                MOUSE_LEN
            }
        }
    }

    /// Nothing held: what a host must hear before the link goes quiet.
    pub fn is_idle(&self) -> bool {
        match *self {
            Report::Keyboard(k) => k == [0; 8],
            Report::Consumer(u) => u == 0,
            Report::Mouse(m) => m.buttons == 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_lay_out_as_the_descriptors_say() {
        let mut b = [0u8; 8];
        assert_eq!(Report::Consumer(0xcd).bytes(&mut b), 2);
        assert_eq!(&b[..2], &[0xcd, 0x00]);
        let m = Mouse {
            buttons: 1,
            dx: -1,
            dy: 5,
            wheel: 0,
            pan: 0,
        };
        assert_eq!(Report::Mouse(m).bytes(&mut b), 5);
        assert_eq!(&b[..5], &[1, 0xff, 5, 0, 0]);
    }
}
