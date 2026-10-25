//! Virtual terminals: each desktop owns one. seatd gives the screen and
//! input to whichever compositor sits on the active VT, so switching people
//! is switching VTs; seatd pauses the old compositor and resumes the new.
//!
//! Tests set NEXA_SESSIOND_VT_FILE: the "active VT" is then a number in that
//! file, and switching writes it.

use std::fs;
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const VT_ACTIVATE: libc::c_ulong = 0x5606;

pub struct Vts {
    fake: Option<PathBuf>,
}

impl Vts {
    pub fn from_env() -> Self {
        Vts { fake: std::env::var_os("NEXA_SESSIOND_VT_FILE").map(PathBuf::from) }
    }

    /// The VT on screen now (0 if unknown).
    pub fn active(&self) -> u32 {
        let text = match &self.fake {
            Some(f) => fs::read_to_string(f).unwrap_or_default(),
            None => fs::read_to_string("/sys/class/tty/tty0/active").unwrap_or_default(),
        };
        text.trim().trim_start_matches("tty").parse().unwrap_or(0)
    }

    /// Puts `vt` on screen and waits (up to 4 s) until it is.
    pub fn activate(&self, vt: u32) -> Result<(), String> {
        if self.active() == vt {
            return Ok(());
        }
        match &self.fake {
            Some(f) => fs::write(f, vt.to_string()).map_err(|e| e.to_string())?,
            None => {
                let tty = fs::OpenOptions::new().read(true).write(true).open("/dev/tty0").map_err(|e| format!("/dev/tty0: {e}"))?;
                // SAFETY: VT_ACTIVATE takes the VT number by value.
                if unsafe { libc::ioctl(tty.as_raw_fd(), VT_ACTIVATE as _, vt as libc::c_ulong) } != 0 {
                    return Err(format!("couldn't switch to VT {vt}: {}", std::io::Error::last_os_error()));
                }
            }
        }
        let until = Instant::now() + Duration::from_secs(4);
        while Instant::now() < until {
            if self.active() == vt {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Err(format!("the screen didn't switch to VT {vt}"))
    }

    /// A clean text screen on a VT that's about to get a desktop: the
    /// moment before the compositor takes it shows a message, not a cursor.
    pub fn prepare(&self, vt: u32, message: &str) {
        if self.fake.is_some() {
            return;
        }
        if let Ok(mut t) = fs::OpenOptions::new().write(true).open(format!("/dev/tty{vt}")) {
            let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
            // SAFETY: TIOCGWINSZ fills a winsize.
            let (rows, cols) = if unsafe { libc::ioctl(t.as_raw_fd(), libc::TIOCGWINSZ, &mut ws) } == 0 && ws.ws_col > 0 {
                (ws.ws_row as usize, ws.ws_col as usize)
            } else {
                (25, 80)
            };
            let pad = cols.saturating_sub(message.chars().count()) / 2;
            let _ = write!(t, "\x1b[2J\x1b[?25l\x1b[{};1H{}\x1b[1m{}\x1b[0m", rows / 2, " ".repeat(pad), message);
        }
    }
}

/// The first VT from 7 up that no session uses (1-6 are the boot console
/// and the text logins).
pub fn free_vt(used: &[u32]) -> Option<u32> {
    (7..=12).find(|v| !used.contains(v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_free_vts() {
        assert_eq!(free_vt(&[1]), Some(7));
        assert_eq!(free_vt(&[1, 7, 8]), Some(9));
        assert_eq!(free_vt(&[7, 8, 9, 10, 11, 12]), None);
    }

    #[test]
    fn fake_vt_file() {
        let f = std::env::temp_dir().join(format!("nexa-vt-{}", std::process::id()));
        fs::write(&f, "1").unwrap();
        let v = Vts { fake: Some(f.clone()) };
        assert_eq!(v.active(), 1);
        v.activate(7).unwrap();
        assert_eq!(v.active(), 7);
        fs::remove_file(f).ok();
    }
}
