//! The kernel's Bluetooth management socket, for the few switches BlueZ
//! has no D-Bus call for: classic Bluetooth (BR/EDR) on or off. Root only.
//! `btmgmt` does the same but waits on a terminal it doesn't have.

use std::time::{Duration, Instant};

const AF_BLUETOOTH: libc::c_int = 31;
const BTPROTO_HCI: libc::c_int = 1;
const HCI_CHANNEL_CONTROL: u16 = 3;
const HCI_DEV_NONE: u16 = 0xffff;

const READ_INFO: u16 = 0x0004;
const SET_POWERED: u16 = 0x0005;
const SET_BREDR: u16 = 0x002a;
const COMMAND_COMPLETE: u16 = 0x0001;
const COMMAND_STATUS: u16 = 0x0002;

/// Current settings bit: powered.
const POWERED: u32 = 1;

#[repr(C)]
struct SockaddrHci {
    family: libc::sa_family_t,
    dev: u16,
    channel: u16,
}

pub struct Mgmt {
    fd: libc::c_int,
}

/// A command for adapter `index`, as the socket takes it.
pub fn encode(op: u16, index: u16, params: &[u8]) -> Vec<u8> {
    let mut m = Vec::with_capacity(6 + params.len());
    m.extend_from_slice(&op.to_le_bytes());
    m.extend_from_slice(&index.to_le_bytes());
    m.extend_from_slice(&(params.len() as u16).to_le_bytes());
    m.extend_from_slice(params);
    m
}

/// The reply to `op` in event `ev`, if it is one: its status and the
/// parameters that follow.
pub fn reply(op: u16, ev: &[u8]) -> Option<(u8, &[u8])> {
    if ev.len() < 9 {
        return None;
    }
    let code = u16::from_le_bytes([ev[0], ev[1]]);
    let of = u16::from_le_bytes([ev[6], ev[7]]);
    if !matches!(code, COMMAND_COMPLETE | COMMAND_STATUS) || of != op {
        return None;
    }
    Some((ev[8], &ev[9..]))
}

impl Mgmt {
    pub fn open() -> std::io::Result<Mgmt> {
        #[cfg(target_os = "linux")]
        let kind = libc::SOCK_RAW | libc::SOCK_CLOEXEC;
        #[cfg(not(target_os = "linux"))]
        let kind = libc::SOCK_RAW;
        // SAFETY: a socket and a bind with a sockaddr built here.
        unsafe {
            let fd = libc::socket(AF_BLUETOOTH, kind, BTPROTO_HCI);
            if fd < 0 {
                return Err(std::io::Error::last_os_error());
            }
            let sa = SockaddrHci {
                family: AF_BLUETOOTH as libc::sa_family_t,
                dev: HCI_DEV_NONE,
                channel: HCI_CHANNEL_CONTROL,
            };
            let len = std::mem::size_of::<SockaddrHci>() as libc::socklen_t;
            if libc::bind(fd, &sa as *const _ as *const libc::sockaddr, len) != 0 {
                let e = std::io::Error::last_os_error();
                libc::close(fd);
                return Err(e);
            }
            Ok(Mgmt { fd })
        }
    }

    /// Sends `op` and waits up to 5 s for its reply. Ok holds the reply's
    /// parameters; a non-zero status is an error.
    fn run(&self, op: u16, index: u16, params: &[u8]) -> std::io::Result<Vec<u8>> {
        let m = encode(op, index, params);
        // SAFETY: a write from a buffer of its own length.
        if unsafe { libc::write(self.fd, m.as_ptr().cast(), m.len()) } < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let end = Instant::now() + Duration::from_secs(5);
        let mut buf = [0u8; 512];
        while Instant::now() < end {
            let mut p = libc::pollfd {
                fd: self.fd,
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: one pollfd on our stack.
            if unsafe { libc::poll(&mut p, 1, 500) } <= 0 {
                continue;
            }
            // SAFETY: a read into a buffer of its own length.
            let n = unsafe { libc::read(self.fd, buf.as_mut_ptr().cast(), buf.len()) };
            if n <= 0 {
                continue;
            }
            if let Some((status, rest)) = reply(op, &buf[..n as usize]) {
                if status != 0 {
                    return Err(std::io::Error::other(format!(
                        "mgmt {op:#06x}: status {status}"
                    )));
                }
                return Ok(rest.to_vec());
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "mgmt timed out",
        ))
    }

    pub fn powered(&self, index: u16) -> std::io::Result<bool> {
        // Address 6, version 1, manufacturer 2, supported 4, then current.
        let info = self.run(READ_INFO, index, &[])?;
        let cur = info
            .get(13..17)
            .ok_or_else(|| std::io::Error::other("short info"))?;
        Ok(u32::from_le_bytes([cur[0], cur[1], cur[2], cur[3]]) & POWERED != 0)
    }

    pub fn set_powered(&self, index: u16, on: bool) -> std::io::Result<()> {
        self.run(SET_POWERED, index, &[on as u8]).map(|_| ())
    }

    pub fn set_bredr(&self, index: u16, on: bool) -> std::io::Result<()> {
        self.run(SET_BREDR, index, &[on as u8]).map(|_| ())
    }
}

impl Drop for Mgmt {
    fn drop(&mut self) {
        // SAFETY: closing the descriptor this struct owns.
        unsafe { libc::close(self.fd) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_and_replies_have_the_kernels_layout() {
        assert_eq!(encode(SET_BREDR, 0, &[0]), [0x2a, 0, 0, 0, 1, 0, 0]);
        // Command Complete for Set BR/EDR on hci0: status 0, settings.
        let ev = [1, 0, 0, 0, 7, 0, 0x2a, 0, 0, 0x81, 0x02, 0, 0];
        assert_eq!(reply(SET_BREDR, &ev), Some((0, &ev[9..])));
        assert_eq!(reply(SET_POWERED, &ev), None, "another command's reply");
        let status = [2, 0, 0, 0, 3, 0, 0x05, 0, 0x0c];
        assert_eq!(reply(SET_POWERED, &status), Some((0x0c, &[][..])));
    }
}
