//! Nexa Mind's voice and ears, both offline.
//!
//! Speaking: nexa-speak (C, Kokoro through onnxruntime) runs as a child,
//! started on the first sentence and kept loaded while you talk (loading
//! takes about two seconds); unloaded after `idle` like the chat model. Its
//! sentence-by-sentence frames go to the desktop unchanged, so speech
//! starts before the whole answer is made. A listener that hangs up (you
//! pressed stop) stops it at once.
//!
//! Hearing: whisper-cli, once per recording, on a 16 kHz mono WAV the
//! desktop sends (at most two minutes). The file is written to the private
//! folder and deleted afterwards.

use crate::catalog::{self, Model};
use crate::config::Config;
use crate::models;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const MAX_TEXT: usize = 64 * 1024;
const MAX_SECONDS: f64 = 120.0;

struct Running {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
    }
}

pub struct Speech {
    cfg: Config,
    threads: usize,
    /// The voice process; also makes speech one sentence stream at a time.
    voice: Mutex<Option<Running>>,
    last_used: Mutex<Instant>,
    /// One recording heard at a time (whisper uses every core).
    ears: Mutex<()>,
    serial: AtomicU64,
}

/// Why a request couldn't be answered: HTTP status and a message.
pub type Refusal = (u16, String);

impl Speech {
    pub fn new(cfg: Config, threads: usize) -> Speech {
        Speech { cfg, threads, voice: Mutex::new(None), last_used: Mutex::new(Instant::now()), ears: Mutex::new(()), serial: AtomicU64::new(0) }
    }

    pub fn speak_args(&self, m: &Model) -> Vec<String> {
        vec![
            "--model".into(), models::path(&self.cfg, m).to_string_lossy().into(),
            "--voices".into(), format!("{}/kokoro-", self.cfg.models_dir().display()),
            "--threads".into(), self.threads.to_string(),
        ]
    }

    fn start_voice(&self, m: &Model) -> Result<Running, String> {
        let log = std::fs::File::create(self.cfg.state_dir.join("nexa-speak.log")).map_err(|e| e.to_string())?;
        let mut cmd = Command::new(&self.cfg.nexa_speak);
        cmd.args(self.speak_args(m)).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(log);
        // Below your apps (as llama-server): the desktop, the helix included,
        // stays smooth while the next sentences are made.
        // SAFETY: setpriority is async-signal-safe; runs in the child before exec.
        unsafe {
            cmd.pre_exec(|| {
                libc::setpriority(libc::PRIO_PROCESS, 0, 5);
                Ok(())
            });
        }
        let mut child = cmd.spawn().map_err(|e| format!("can't start {}: {e}", self.cfg.nexa_speak.display()))?;
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Ok(Running { child, stdin, stdout })
    }

    /// Check a request before anything is sent back.
    pub fn check_speak(&self, text: &str, voice: &str, speed: f64) -> Result<&'static Model, Refusal> {
        let m = catalog::find(catalog::DEFAULT_VOICE).ok_or((500, "no voice model in the catalogue".to_string()))?;
        if !models::is_installed(&self.cfg, m) {
            return Err((409, format!("model '{}' is not installed", m.name)));
        }
        if text.trim().is_empty() || text.len() > MAX_TEXT {
            return Err((400, format!("text must be 1 to {MAX_TEXT} bytes")));
        }
        if catalog::voice(voice).is_none() {
            return Err((400, format!("no voice named '{voice}'")));
        }
        if !(0.5..=2.0).contains(&speed) {
            return Err((400, "speed must be 0.5 to 2".into()));
        }
        Ok(m)
    }

    /// Speak `text`, copying nexa-speak's frames to `out` (after `head` has
    /// written the response's start). Errors after the head are logged only.
    pub fn speak(&self, m: &Model, text: &str, voice: &str, speed: f64, out: &mut impl Write,
                 head: impl FnOnce(&mut dyn Write) -> std::io::Result<()>) -> std::io::Result<()> {
        let mut slot = self.voice.lock().unwrap();
        let alive = slot.as_mut().map_or(false, |r| matches!(r.child.try_wait(), Ok(None)));
        if !alive {
            *slot = None;
            match self.start_voice(m) {
                Ok(r) => *slot = Some(r),
                Err(e) => {
                    eprintln!("nexa-mindd: {e}");
                    return crate::http::respond_json(out, 503, &serde_json::json!({"error": e}));
                }
            }
        }
        let r = slot.as_mut().unwrap();
        let sent = write!(r.stdin, "{voice} {speed:.2} {}\n", text.len())
            .and_then(|_| r.stdin.write_all(text.as_bytes()))
            .and_then(|_| r.stdin.flush());
        if let Err(e) = sent {
            *slot = None;
            return crate::http::respond_json(out, 503, &serde_json::json!({"error": format!("the voice stopped: {e}")}));
        }
        head(out)?;
        let result = copy_frames(&mut r.stdout, out);
        *self.last_used.lock().unwrap() = Instant::now();
        match result {
            Ok(()) => Ok(()),
            Err(e) => {
                // A listener that went away, or a voice that died mid-way:
                // either way the stream is out of step; start afresh next time.
                *slot = None;
                Err(e)
            }
        }
    }

    /// Load the voice ahead of the answer (you started talking to Nexa), so
    /// its first sentence comes in about a second, not after loading. No-op
    /// if it is loaded or not installed.
    pub fn warm(&self) {
        let Some(m) = catalog::find(catalog::DEFAULT_VOICE) else { return };
        if !models::is_installed(&self.cfg, m) {
            return;
        }
        let mut slot = self.voice.lock().unwrap();
        let alive = slot.as_mut().map_or(false, |r| matches!(r.child.try_wait(), Ok(None)));
        if !alive {
            match self.start_voice(m) {
                Ok(r) => *slot = Some(r),
                Err(e) => eprintln!("nexa-mindd: warm-up: {e}"),
            }
        }
        *self.last_used.lock().unwrap() = Instant::now();
    }

    /// Unload the voice after `idle` without speech. True if it was unloaded.
    pub fn reap_if_idle(&self) -> bool {
        let Ok(mut slot) = self.voice.try_lock() else { return false };
        if slot.is_some() && self.last_used.lock().unwrap().elapsed() >= self.cfg.idle {
            *slot = None;
            return true;
        }
        false
    }

    pub fn voice_loaded(&self) -> bool {
        self.voice.try_lock().map(|s| s.is_some()).unwrap_or(true)
    }

    /// Words in a recording (WAV, 16 kHz mono 16-bit).
    pub fn hear(&self, wav: &[u8]) -> Result<String, Refusal> {
        let m = catalog::find(catalog::DEFAULT_LISTEN).ok_or((500, "no speech model in the catalogue".to_string()))?;
        if !models::is_installed(&self.cfg, m) {
            return Err((409, format!("model '{}' is not installed", m.name)));
        }
        let secs = wav_seconds(wav).map_err(|e| (400, e))?;
        if secs > MAX_SECONDS {
            return Err((400, format!("recordings are at most {MAX_SECONDS} seconds")));
        }
        let _one = self.ears.lock().unwrap();
        let n = self.serial.fetch_add(1, Ordering::SeqCst);
        let file = self.cfg.private_dir().join(format!("heard-{}-{n}.wav", std::process::id()));
        {
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&file)
                .map_err(|e| (500, e.to_string()))?;
            f.write_all(wav).map_err(|e| (500, e.to_string()))?;
        }
        let result = self.run_whisper(m, &file, secs);
        std::fs::remove_file(&file).ok();
        result
    }

    pub fn whisper_args(&self, m: &Model, file: &std::path::Path) -> Vec<String> {
        vec![
            "-m".into(), models::path(&self.cfg, m).to_string_lossy().into(),
            "-f".into(), file.to_string_lossy().into(),
            "-l".into(), "en".into(),
            "-t".into(), self.threads.to_string(),
            "-nt".into(), "-np".into(), "-sns".into(), "-ng".into(),
        ]
    }

    fn run_whisper(&self, m: &Model, file: &std::path::Path, secs: f64) -> Result<String, Refusal> {
        let mut child = Command::new(&self.cfg.whisper)
            .args(self.whisper_args(m, file))
            .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null())
            .spawn().map_err(|e| (503, format!("can't start {}: {e}", self.cfg.whisper.display())))?;
        let mut stdout = child.stdout.take().unwrap();
        let reader = std::thread::spawn(move || {
            let mut s = String::new();
            stdout.read_to_string(&mut s).ok();
            s
        });
        // Generous: a slow laptop hears about five times faster than speech.
        let deadline = Instant::now() + Duration::from_secs_f64(30.0 + secs * 2.0);
        let status = loop {
            match child.try_wait() {
                Ok(Some(st)) => break st,
                Ok(None) if Instant::now() > deadline => {
                    child.kill().ok();
                    child.wait().ok();
                    return Err((504, "hearing took too long".into()));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(e) => return Err((500, e.to_string())),
            }
        };
        let text = reader.join().unwrap_or_default();
        if !status.success() {
            return Err((500, format!("whisper failed ({status})")));
        }
        Ok(tidy_transcript(&text))
    }
}

/// Copy frames until END or ERR. Frame: "C <rate> <samples> <text bytes>\n"
/// then the text and samples x 2 bytes (see tools-c/nexa-speak).
pub fn copy_frames(from: &mut impl BufRead, to: &mut impl Write) -> std::io::Result<()> {
    let bad = |what: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("voice: {what}"));
    loop {
        let mut line = String::new();
        if from.read_line(&mut line)? == 0 {
            return Err(bad("stopped mid-answer"));
        }
        if line.len() > 200 || !line.ends_with('\n') {
            return Err(bad("bad frame"));
        }
        to.write_all(line.as_bytes())?;
        if line == "END\n" || line.starts_with("ERR ") {
            return to.flush();
        }
        let f: Vec<&str> = line.trim_end().split(' ').collect();
        let (samples, text) = match f.as_slice() {
            ["C", _rate, s, t] => (s.parse::<u64>().map_err(|_| bad("bad frame"))?, t.parse::<u64>().map_err(|_| bad("bad frame"))?),
            _ => return Err(bad("bad frame")),
        };
        if text > MAX_TEXT as u64 || samples > 24_000 * 120 {
            return Err(bad("frame too big"));
        }
        let n = text + samples * 2;
        let copied = std::io::copy(&mut from.by_ref().take(n), to)?;
        if copied != n {
            return Err(bad("stopped mid-frame"));
        }
        to.flush()?;
    }
}

/// Length of a 16 kHz mono 16-bit PCM WAV, or why it isn't one.
pub fn wav_seconds(b: &[u8]) -> Result<f64, String> {
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err("not a WAV file".into());
    }
    let u16_at = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    let (mut i, mut fmt_ok) = (12usize, false);
    while i + 8 <= b.len() {
        let (id, len) = (&b[i..i + 4], u32_at(i + 4) as usize);
        let body = i + 8;
        if id == b"fmt " {
            if len < 16 || body + 16 > b.len() {
                return Err("bad WAV header".into());
            }
            fmt_ok = u16_at(body) == 1 && u16_at(body + 2) == 1 && u32_at(body + 4) == 16_000 && u16_at(body + 14) == 16;
            if !fmt_ok {
                return Err("WAV must be 16 kHz mono 16-bit PCM".into());
            }
        } else if id == b"data" {
            if !fmt_ok {
                return Err("bad WAV header".into());
            }
            let len = len.min(b.len() - body);
            let secs = len as f64 / 32_000.0;
            if secs < 0.2 {
                return Err("too short to hear".into());
            }
            return Ok(secs);
        }
        i = body + len + (len & 1);
    }
    Err("bad WAV header".into())
}

/// whisper's lines as one sentence, without its notes for non-speech
/// ("[BLANK_AUDIO]", "(wind blowing)").
pub fn tidy_transcript(raw: &str) -> String {
    let mut out = String::new();
    let (mut depth, mut starred) = (0i32, false);
    for c in raw.chars() {
        match c {
            '*' => starred = !starred,
            '[' | '(' => depth += 1,
            ']' | ')' if depth > 0 => depth -= 1,
            _ if depth > 0 || starred => {}
            '\n' | '\r' | '\t' => out.push(' '),
            _ => out.push(c),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(rate: u32, channels: u16, bits: u16, samples: usize) -> Vec<u8> {
        let data = samples * channels as usize * bits as usize / 8;
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&((36 + data) as u32).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&channels.to_le_bytes());
        v.extend_from_slice(&rate.to_le_bytes());
        v.extend_from_slice(&(rate * channels as u32 * bits as u32 / 8).to_le_bytes());
        v.extend_from_slice(&(channels * bits / 8).to_le_bytes());
        v.extend_from_slice(&bits.to_le_bytes());
        v.extend_from_slice(b"LIST\x04\0\0\0abcd");   // a chunk to skip
        v.extend_from_slice(b"data");
        v.extend_from_slice(&(data as u32).to_le_bytes());
        v.resize(v.len() + data, 0);
        v
    }

    #[test]
    fn recordings() {
        assert_eq!(wav_seconds(&wav(16_000, 1, 16, 32_000)), Ok(2.0));
        assert!(wav_seconds(&wav(44_100, 1, 16, 32_000)).unwrap_err().contains("16 kHz"));
        assert!(wav_seconds(&wav(16_000, 2, 16, 32_000)).is_err(), "stereo");
        assert!(wav_seconds(&wav(16_000, 1, 8, 32_000)).is_err(), "8-bit");
        assert!(wav_seconds(&wav(16_000, 1, 16, 1000)).unwrap_err().contains("short"));
        assert!(wav_seconds(b"RIFF\0\0\0\0WAVE").is_err());
        assert!(wav_seconds(b"hello").is_err());
        let mut cut = wav(16_000, 1, 16, 32_000);
        cut.truncate(56 + 16_000);
        assert_eq!(wav_seconds(&cut), Ok(0.5), "a short read counts what's there");
    }

    #[test]
    fn transcripts() {
        assert_eq!(tidy_transcript(" Open my mail.\n And the music.\n"), "Open my mail. And the music.");
        assert_eq!(tidy_transcript("[BLANK_AUDIO]\n"), "");
        assert_eq!(tidy_transcript("(wind blowing) Hello [Music] there *coughs*"), "Hello there");
    }

    #[test]
    fn frames_pass_through_whole() {
        let mut src = Vec::new();
        src.extend_from_slice(b"C 24000 3 5\nHello");
        src.extend_from_slice(&[1, 0, 2, 0, 3, 0]);
        src.extend_from_slice(b"END\n");
        src.extend_from_slice(b"C 24000 1 1\nX\0\0");  // the next answer: not read
        let mut r = std::io::Cursor::new(src.clone());
        let mut out = Vec::new();
        copy_frames(&mut r, &mut out).unwrap();
        assert_eq!(out, src[..src.len() - 15]);
        let mut r = std::io::Cursor::new(b"ERR no voice named x\n".to_vec());
        let mut out = Vec::new();
        copy_frames(&mut r, &mut out).unwrap();
        assert_eq!(out, b"ERR no voice named x\n");
        for broken in [&b"C 24000 3 5\nHel"[..], b"", b"garbage\n", b"C 24000 999999999999 0\n"] {
            let mut r = std::io::Cursor::new(broken.to_vec());
            assert!(copy_frames(&mut r, &mut Vec::new()).is_err(), "{:?}", String::from_utf8_lossy(broken));
        }
    }
}
