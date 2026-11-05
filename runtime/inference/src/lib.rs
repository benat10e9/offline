//! llama-server supervision. The model runs in its own process so a crash or an
//! out-of-memory kill never takes down the app, and so we can ship one inference
//! build per GPU backend without rebuilding the app.
//!
//! SSD layout (see spec/ssd-layout.md):
//!   bin/<platform>/<backend>/llama-server[.exe]   e.g. bin/macos-aarch64/metal/llama-server
//!   models/models.json                            catalog of GGUF files with RAM floors

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum InferenceError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("model catalog: {0}")]
    Catalog(#[from] serde_json::Error),
    #[error("no llama-server binary found for {0}")]
    NoBinary(String),
    #[error("llama-server did not become healthy within {0:?}")]
    StartTimeout(Duration),
    #[error("llama-server exited during startup with status {0}")]
    ExitedEarly(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelEntry {
    pub id: String,
    /// Path relative to the models directory.
    pub file: String,
    /// Minimum total system RAM (GB) at which we offer this model.
    pub min_ram_gb: u32,
    pub context: u32,
    /// SPDX id of the weights license. Only commercially redistributable licenses ship.
    pub license: String,
    /// Passed the safety eval for generated answers.
    #[serde(default)]
    pub safety_approved: bool,
    /// SHA-256 of the GGUF file, recorded at provisioning.
    #[serde(default)]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCatalog {
    pub models: Vec<ModelEntry>,
}

impl ModelCatalog {
    pub fn load(path: &Path) -> Result<Self, InferenceError> {
        Ok(serde_json::from_slice(&std::fs::read(path)?)?)
    }
}

/// What this machine gets. Below the floor, the app runs search-and-read only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tier {
    SearchOnly { ram_gb: u32 },
    Generate { ram_gb: u32, model: ModelEntry },
}

pub fn total_ram_gb() -> u32 {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    (sys.total_memory() / (1024 * 1024 * 1024)) as u32
}

/// Pick the largest safety-approved model whose RAM floor fits. Deterministic.
pub fn select_tier(ram_gb: u32, catalog: &ModelCatalog) -> Tier {
    catalog
        .models
        .iter()
        .filter(|m| m.safety_approved && m.min_ram_gb <= ram_gb)
        .max_by_key(|m| m.min_ram_gb)
        .map(|m| Tier::Generate { ram_gb, model: m.clone() })
        .unwrap_or(Tier::SearchOnly { ram_gb })
}

pub fn platform_dir() -> String {
    let os = match std::env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        _ => "linux",
    };
    format!("{os}-{}", std::env::consts::ARCH)
}

/// Backends to try, best first. A backend whose binary is missing or that exits
/// at startup is skipped, so a machine without CUDA falls back to Vulkan, then CPU.
pub fn backend_preference() -> Vec<&'static str> {
    match std::env::consts::OS {
        "macos" => vec!["metal", "cpu"],
        _ => vec!["cuda", "vulkan", "cpu"],
    }
}

pub fn binary_name() -> &'static str {
    if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    }
}

/// Candidate binaries under `bin_root`, in preference order, that exist on disk.
pub fn candidate_binaries(bin_root: &Path) -> Vec<(String, PathBuf)> {
    let plat = platform_dir();
    // Absolute: the server is started with its own folder as the working directory.
    let bin_root = std::fs::canonicalize(bin_root).unwrap_or_else(|_| bin_root.to_path_buf());
    backend_preference()
        .into_iter()
        .map(|b| (b.to_string(), bin_root.join(&plat).join(b).join(binary_name())))
        .filter(|(_, p)| p.is_file())
        .collect()
}

pub fn free_local_port() -> Result<u16, InferenceError> {
    Ok(TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

/// GET /health on 127.0.0.1:port. Plain TCP so no proxy settings can interfere.
pub fn health_ok(port: u16) -> bool {
    let Ok(mut s) = TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(300)) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_millis(1000)));
    if s.write_all(b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").is_err() {
        return false;
    }
    let mut buf = [0u8; 64];
    match s.read(&mut buf) {
        Ok(n) => std::str::from_utf8(&buf[..n])
            .map(|t| t.starts_with("HTTP/1.1 200") || t.starts_with("HTTP/1.0 200"))
            .unwrap_or(false),
        Err(_) => false,
    }
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub model_path: PathBuf,
    pub context: u32,
    pub gpu_layers: u32,
    pub startup_timeout: Duration,
    pub extra_args: Vec<String>,
}

pub fn server_args(cfg: &ServerConfig, port: u16) -> Vec<String> {
    let mut a = vec![
        "-m".into(),
        cfg.model_path.to_string_lossy().into_owned(),
        "--host".into(),
        "127.0.0.1".into(),
        "--port".into(),
        port.to_string(),
        "-c".into(),
        cfg.context.to_string(),
        "-ngl".into(),
        cfg.gpu_layers.to_string(),
    ];
    a.extend(cfg.extra_args.iter().cloned());
    a
}

/// A running llama-server. Killed when dropped.
pub struct Supervisor {
    child: Child,
    pub port: u16,
    pub backend: String,
    pub binary: PathBuf,
    cfg: ServerConfig,
    pub restarts: u32,
}

impl Supervisor {
    /// Try each available backend in order until one starts and reports healthy.
    pub fn start(bin_root: &Path, cfg: ServerConfig) -> Result<Self, InferenceError> {
        let candidates = candidate_binaries(bin_root);
        if candidates.is_empty() {
            return Err(InferenceError::NoBinary(platform_dir()));
        }
        let mut last = None;
        for (backend, bin) in candidates {
            match Self::spawn(&bin, &cfg) {
                Ok((child, port)) => {
                    return Ok(Self { child, port, backend, binary: bin, cfg, restarts: 0 });
                }
                Err(e) => last = Some(e),
            }
        }
        Err(last.expect("at least one candidate was tried"))
    }

    fn spawn(bin: &Path, cfg: &ServerConfig) -> Result<(Child, u16), InferenceError> {
        let port = free_local_port()?;
        let mut cfg = cfg.clone();
        cfg.model_path = std::fs::canonicalize(&cfg.model_path).unwrap_or(cfg.model_path);
        let cfg = &cfg;
        let mut cmd = Command::new(bin);
        cmd.args(server_args(cfg, port)).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        if let Some(dir) = bin.parent() {
            cmd.current_dir(dir); // shared libraries ship beside the binary
        }
        let mut child = cmd.spawn()?;
        let deadline = Instant::now() + cfg.startup_timeout;
        while Instant::now() < deadline {
            if let Some(status) = child.try_wait()? {
                return Err(InferenceError::ExitedEarly(status.to_string()));
            }
            if health_ok(port) {
                return Ok((child, port));
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        let _ = child.kill();
        let _ = child.wait();
        Err(InferenceError::StartTimeout(cfg.startup_timeout))
    }

    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Call before each request: restarts the server if it died. Returns false if
    /// it could not be brought back, in which case the app drops to search-only.
    pub fn ensure_running(&mut self, max_restarts: u32) -> bool {
        let alive = matches!(self.child.try_wait(), Ok(None)) && health_ok(self.port);
        if alive {
            return true;
        }
        if self.restarts >= max_restarts {
            return false;
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.restarts += 1;
        match Self::spawn(&self.binary, &self.cfg) {
            Ok((child, port)) => {
                self.child = child;
                self.port = port;
                true
            }
            Err(_) => false,
        }
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(id: &str, ram: u32, ok: bool) -> ModelEntry {
        ModelEntry {
            id: id.into(),
            file: format!("{id}.gguf"),
            min_ram_gb: ram,
            context: 8192,
            license: "Apache-2.0".into(),
            safety_approved: ok,
            sha256: None,
        }
    }

    #[test]
    fn tiers() {
        let c = ModelCatalog { models: vec![m("small", 16, true), m("mid", 32, true), m("unsafe-tiny", 8, false)] };
        assert_eq!(select_tier(8, &c), Tier::SearchOnly { ram_gb: 8 });
        assert!(matches!(select_tier(16, &c), Tier::Generate { model, .. } if model.id == "small"));
        assert!(matches!(select_tier(64, &c), Tier::Generate { model, .. } if model.id == "mid"));
    }

    #[test]
    fn args_bind_localhost_only() {
        let cfg = ServerConfig {
            model_path: "models/x.gguf".into(),
            context: 8192,
            gpu_layers: 99,
            startup_timeout: Duration::from_secs(1),
            extra_args: vec![],
        };
        let a = server_args(&cfg, 5555);
        let i = a.iter().position(|x| x == "--host").unwrap();
        assert_eq!(a[i + 1], "127.0.0.1");
    }

    #[test]
    fn binary_discovery_respects_preference() {
        let root = tempfile::tempdir().unwrap();
        let plat = root.path().join(platform_dir());
        for b in ["cpu", backend_preference()[0]] {
            std::fs::create_dir_all(plat.join(b)).unwrap();
            std::fs::write(plat.join(b).join(binary_name()), b"").unwrap();
        }
        let c = candidate_binaries(root.path());
        assert_eq!(c[0].0, backend_preference()[0]);
        assert_eq!(c.last().unwrap().0, "cpu");
    }

    #[test]
    fn health_check_against_fake_server() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = l.accept() {
                let mut b = [0u8; 256];
                let _ = s.read(&mut b);
                let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}");
            }
        });
        assert!(health_ok(port));
        assert!(!health_ok(free_local_port().unwrap()));
    }

    #[cfg(unix)]
    #[test]
    fn starts_from_a_relative_root_with_relative_model() {
        // A fake llama-server: checks the model path it was given exists, then serves /health.
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let prev = std::env::current_dir().unwrap();
        let bin = dir.path().join("bin").join(platform_dir()).join("cpu");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(dir.path().join("models")).unwrap();
        std::fs::write(dir.path().join("models/m.gguf"), b"x").unwrap();
        let script = r#"#!/bin/sh
model=$2; port=$6
[ -f "$model" ] || exit 7
exec python3 -c "
import http.server,sys
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(s): s.send_response(200); s.end_headers(); s.wfile.write(b'{}')
    def log_message(*a): pass
http.server.HTTPServer(('127.0.0.1', int(sys.argv[1])), H).serve_forever()" "$port"
"#;
        let exe = bin.join(binary_name());
        std::fs::write(&exe, script).unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let cfg = ServerConfig {
            model_path: "models/m.gguf".into(),
            context: 1,
            gpu_layers: 0,
            startup_timeout: Duration::from_secs(20),
            extra_args: vec![],
        };
        let result = Supervisor::start(Path::new("bin"), cfg);
        std::env::set_current_dir(prev).unwrap();
        let sup = result.expect("relative root and model path must work");
        assert!(health_ok(sup.port));
    }

    #[test]
    fn missing_binary_is_reported() {
        let root = tempfile::tempdir().unwrap();
        let cfg = ServerConfig {
            model_path: "x".into(),
            context: 1,
            gpu_layers: 0,
            startup_timeout: Duration::from_millis(10),
            extra_args: vec![],
        };
        assert!(matches!(Supervisor::start(root.path(), cfg), Err(InferenceError::NoBinary(_))));
    }
}
