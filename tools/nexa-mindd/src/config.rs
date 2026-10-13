//! Where things live and how the service behaves. Every path can be moved
//! with an environment variable, which the tests use.

use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Config {
    /// Models and tuning results (owned by the nexa-mind user).
    pub state_dir: PathBuf,
    /// Sockets: mind.sock (for the desktop) and private/llm.sock.
    pub run_dir: PathBuf,
    pub llama_server: PathBuf,
    pub curl: PathBuf,
    /// Unload the model after this long without a request.
    pub idle: Duration,
    pub context: u32,
    /// Where /sys and /proc are read from (tests point this elsewhere).
    pub sys_root: PathBuf,
}

fn env_path(name: &str, default: &str) -> PathBuf {
    std::env::var_os(name).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(default))
}

impl Config {
    pub fn from_env() -> Config {
        let idle = std::env::var("NEXA_MIND_IDLE_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(600);
        Config {
            state_dir: env_path("NEXA_MIND_STATE", "/var/lib/nexa-mind"),
            run_dir: env_path("NEXA_MIND_RUN", "/run/nexa-mind"),
            llama_server: env_path("NEXA_MIND_LLAMA_SERVER", "/usr/bin/llama-server"),
            curl: env_path("NEXA_MIND_CURL", "/usr/bin/curl"),
            idle: Duration::from_secs(idle),
            context: 8192,
            sys_root: env_path("NEXA_MIND_SYS_ROOT", "/"),
        }
    }
    pub fn models_dir(&self) -> PathBuf {
        self.state_dir.join("models")
    }
    pub fn public_socket(&self) -> PathBuf {
        self.run_dir.join("mind.sock")
    }
    pub fn private_dir(&self) -> PathBuf {
        self.run_dir.join("private")
    }
    pub fn llm_socket(&self) -> PathBuf {
        self.private_dir().join("llm.sock")
    }
}
