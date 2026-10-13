//! Supervises llama-server: started on the first question, unloaded after
//! `idle` without requests, restarted if it dies. It listens only on a Unix
//! socket in a folder no one else can enter, and runs at lower CPU priority
//! than your apps (nice 5).

use crate::catalog::Model;
use crate::config::Config;
use crate::{cpu, http, models};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Idle,
    Loading,
    Ready,
}

struct Inner {
    child: Option<Child>,
    model: Option<&'static str>,
    state: State,
    last_used: Instant,
}

pub struct Supervisor {
    cfg: Config,
    /// Brief state access only (status must answer while a model loads).
    inner: Mutex<Inner>,
    /// Held while (re)starting the model server, so two requests never
    /// start two servers.
    load: Mutex<()>,
    active: AtomicUsize,
    pub threads: usize,
    pub threads_batch: usize,
}

/// Counts a request as in flight; the model is never unloaded under one.
pub struct InUse<'a>(&'a Supervisor);

impl Drop for InUse<'_> {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
        self.0.inner.lock().unwrap().last_used = Instant::now();
    }
}

impl Supervisor {
    pub fn new(cfg: Config) -> Supervisor {
        let threads = cpu::physical_cores(&cfg.sys_root).max(1);
        let threads_batch = cpu::logical_cpus(&cfg.sys_root).max(threads);
        Supervisor {
            cfg,
            inner: Mutex::new(Inner { child: None, model: None, state: State::Idle, last_used: Instant::now() }),
            load: Mutex::new(()),
            active: AtomicUsize::new(0),
            threads,
            threads_batch,
        }
    }

    pub fn status(&self) -> (State, Option<&'static str>) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(child) = inner.child.as_mut() {
            if !matches!(child.try_wait(), Ok(None)) {
                inner.child = None;
                inner.state = State::Idle;
                inner.model = None;
            }
        }
        (inner.state, inner.model)
    }

    pub fn args(&self, m: &Model) -> Vec<String> {
        let s = |v: &str| v.to_string();
        vec![
            s("--model"), models::path(&self.cfg, m).to_string_lossy().into(),
            s("--host"), self.cfg.llm_socket().to_string_lossy().into(),
            s("--threads"), self.threads.to_string(),
            s("--threads-batch"), self.threads_batch.to_string(),
            s("--ctx-size"), self.cfg.context.to_string(),
            s("--parallel"), s("1"),
            s("--flash-attn"), s("auto"),
            s("--cache-type-k"), s("q8_0"),
            s("--cache-type-v"), s("q8_0"),
            s("--cache-reuse"), s("256"),
            s("--jinja"),
            s("--no-webui"),
        ]
    }

    /// Make sure `m` is loaded and answering; returns a guard that keeps it
    /// loaded until dropped.
    pub fn acquire(&self, m: &'static Model) -> Result<InUse<'_>, String> {
        if !models::is_installed(&self.cfg, m) {
            return Err(format!("model '{}' is not installed", m.name));
        }
        self.active.fetch_add(1, Ordering::SeqCst);
        let guard = InUse(self);
        let _loading = self.load.lock().unwrap();
        let mut inner = self.inner.lock().unwrap();
        let alive = inner.child.as_mut().map_or(false, |c| matches!(c.try_wait(), Ok(None)));
        if alive && inner.model == Some(m.name) && inner.state == State::Ready {
            return Ok(guard);
        }
        if let Some(mut old) = inner.child.take() {
            old.kill().ok();
            old.wait().ok();
        }
        std::fs::create_dir_all(self.cfg.private_dir()).map_err(|e| e.to_string())?;
        std::fs::remove_file(self.cfg.llm_socket()).ok();
        let log = std::fs::File::create(self.cfg.state_dir.join("llama-server.log")).map_err(|e| e.to_string())?;
        let mut cmd = Command::new(&self.cfg.llama_server);
        cmd.args(self.args(m)).stdin(Stdio::null()).stdout(log.try_clone().map_err(|e| e.to_string())?).stderr(log);
        // SAFETY: setpriority is async-signal-safe; runs in the child before exec.
        unsafe {
            cmd.pre_exec(|| {
                libc::setpriority(libc::PRIO_PROCESS, 0, 5);
                Ok(())
            });
        }
        let child = cmd.spawn().map_err(|e| format!("can't start {}: {e}", self.cfg.llama_server.display()))?;
        inner.child = Some(child);
        inner.model = Some(m.name);
        inner.state = State::Loading;
        drop(inner);
        // Wait for the model to load (llama-server answers /health with 200
        // once ready, 503 while loading). The state lock is free meanwhile.
        let started = Instant::now();
        loop {
            {
                let mut inner = self.inner.lock().unwrap();
                if let Some(c) = inner.child.as_mut() {
                    if let Ok(Some(status)) = c.try_wait() {
                        inner.child = None;
                        inner.state = State::Idle;
                        inner.model = None;
                        return Err(format!("the model server stopped while loading ({status}); see llama-server.log"));
                    }
                }
            }
            if let Ok(s) = http::send(&self.cfg.llm_socket(), "GET", "/health", b"") {
                if matches!(http::read_response(s), Ok((200, _))) {
                    break;
                }
            }
            if started.elapsed() > Duration::from_secs(300) {
                return Err("the model took too long to load".into());
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        let mut inner = self.inner.lock().unwrap();
        inner.state = State::Ready;
        inner.last_used = Instant::now();
        Ok(guard)
    }

    pub fn unload(&self) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(mut c) = inner.child.take() {
            c.kill().ok();
            c.wait().ok();
        }
        inner.state = State::Idle;
        inner.model = None;
    }

    /// Called periodically: unload once nothing has used the model for `idle`.
    pub fn reap_if_idle(&self) -> bool {
        let idle_long = {
            let inner = self.inner.lock().unwrap();
            inner.child.is_some() && self.active.load(Ordering::SeqCst) == 0 && inner.last_used.elapsed() >= self.cfg.idle
        };
        if idle_long {
            self.unload();
        }
        idle_long
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_keep_the_model_private_and_tuned() {
        let mut cfg = Config::from_env();
        cfg.run_dir = "/run/nexa-mind".into();
        let sup = Supervisor::new(cfg);
        let m = crate::catalog::find(crate::catalog::DEFAULT_CHAT).unwrap();
        let args = sup.args(m);
        let after = |flag: &str| args[args.iter().position(|a| a == flag).unwrap() + 1].clone();
        assert_eq!(after("--host"), "/run/nexa-mind/private/llm.sock", "Unix socket only, never a TCP port");
        assert_eq!(after("--threads"), sup.threads.to_string());
        assert!(sup.threads <= sup.threads_batch);
        assert!(args.contains(&"--jinja".to_string()), "tool calling needs the chat template");
    }

    #[test]
    fn refuses_a_model_that_isnt_installed() {
        let mut cfg = Config::from_env();
        cfg.state_dir = std::env::temp_dir().join(format!("nexa-sup-{}", std::process::id()));
        let sup = Supervisor::new(cfg);
        let m = crate::catalog::find(crate::catalog::DEFAULT_CHAT).unwrap();
        assert!(sup.acquire(m).err().unwrap().contains("not installed"));
        assert_eq!(sup.active.load(Ordering::SeqCst), 0, "a refused request doesn't stay counted");
    }
}
