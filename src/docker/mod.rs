use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use bollard::{Docker, API_DEFAULT_VERSION};
use serde::Deserialize;

pub mod containers;
pub mod images;
pub mod networks;
pub mod volumes;

const SOCKET_TIMEOUT_SECS: u64 = 120;

#[derive(Debug, Clone)]
pub struct DaemonInfo {
    pub server_version: String,
    pub socket: String,
}

#[derive(Debug, Clone)]
pub struct DockerClient {
    inner: Docker,
    pub info: DaemonInfo,
}

impl DockerClient {
    pub fn raw(&self) -> &Docker {
        &self.inner
    }
}

#[derive(Debug, Deserialize)]
struct ContextMetaEndpoints {
    docker: Option<ContextMetaDocker>,
}

#[derive(Debug, Deserialize)]
struct ContextMetaDocker {
    #[serde(rename = "Host")]
    host: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ContextMeta {
    #[serde(rename = "Endpoints")]
    endpoints: Option<ContextMetaEndpoints>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SocketSource {
    CliFlag(String),
    EnvVar(String),
    DockerContext(String),
    ColimaDefault(String),
    SystemDefault(String),
}

impl SocketSource {
    pub fn path(&self) -> &str {
        match self {
            Self::CliFlag(p)
            | Self::EnvVar(p)
            | Self::DockerContext(p)
            | Self::ColimaDefault(p)
            | Self::SystemDefault(p) => p,
        }
    }
}

pub trait SocketEnv {
    fn host_flag(&self) -> Option<String>;
    fn env_docker_host(&self) -> Option<String>;
    fn home(&self) -> Option<PathBuf>;
    fn path_exists(&self, p: &str) -> bool;
    fn read_to_string(&self, p: &Path) -> Option<String>;
    fn list_dir(&self, p: &Path) -> Vec<PathBuf>;
}

pub struct RealEnv {
    pub host_flag: Option<String>,
}

impl SocketEnv for RealEnv {
    fn host_flag(&self) -> Option<String> {
        self.host_flag.clone()
    }
    fn env_docker_host(&self) -> Option<String> {
        std::env::var("DOCKER_HOST").ok().filter(|s| !s.is_empty())
    }
    fn home(&self) -> Option<PathBuf> {
        dirs::home_dir()
    }
    fn path_exists(&self, p: &str) -> bool {
        std::path::Path::new(p).exists()
    }
    fn read_to_string(&self, p: &Path) -> Option<String> {
        std::fs::read_to_string(p).ok()
    }
    fn list_dir(&self, p: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(p)
            .ok()
            .into_iter()
            .flat_map(|rd| rd.flatten().map(|e| e.path()))
            .collect()
    }
}

pub fn resolve_socket(env: &dyn SocketEnv) -> Result<SocketSource> {
    if let Some(host) = env.host_flag() {
        return Ok(SocketSource::CliFlag(host));
    }
    if let Some(host) = env.env_docker_host() {
        return Ok(SocketSource::EnvVar(host));
    }
    if let Some(host) = active_context_socket(env) {
        return Ok(SocketSource::DockerContext(host));
    }
    if let Some(home) = env.home() {
        let colima = home.join(".colima/default/docker.sock");
        let colima_str = colima.to_string_lossy().to_string();
        if env.path_exists(&colima_str) {
            return Ok(SocketSource::ColimaDefault(format!(
                "unix://{}",
                colima_str
            )));
        }
    }
    let system = "/var/run/docker.sock";
    if env.path_exists(system) {
        return Ok(SocketSource::SystemDefault(format!("unix://{}", system)));
    }
    anyhow::bail!(
        "no docker socket found — set DOCKER_HOST, pass --host, or start colima with `colima start`"
    )
}

fn active_context_socket(env: &dyn SocketEnv) -> Option<String> {
    let home = env.home()?;
    let meta_root = home.join(".docker/contexts/meta");
    let entries = env.list_dir(&meta_root);
    for entry in entries {
        let meta_file = entry.join("meta.json");
        let Some(raw) = env.read_to_string(&meta_file) else {
            continue;
        };
        let parsed: serde_json::Result<ContextMeta> = serde_json::from_str(&raw);
        let Ok(meta) = parsed else { continue };
        let host = meta.endpoints?.docker?.host?;
        if !host.is_empty() {
            return Some(host);
        }
    }
    None
}

pub async fn connect(host_flag: Option<String>) -> Result<DockerClient> {
    let env = RealEnv { host_flag };
    let src = resolve_socket(&env)?;
    let inner = build_client(src.path())?;
    let version = inner
        .version()
        .await
        .context("failed to query docker daemon version")?;
    let info = DaemonInfo {
        server_version: version.version.unwrap_or_else(|| "unknown".into()),
        socket: src.path().to_string(),
    };
    Ok(DockerClient { inner, info })
}

fn build_client(host: &str) -> Result<Docker> {
    if let Some(path) = host.strip_prefix("unix://") {
        Ok(Docker::connect_with_socket(
            path,
            SOCKET_TIMEOUT_SECS,
            API_DEFAULT_VERSION,
        )?)
    } else if host.starts_with("tcp://") || host.starts_with("http://") {
        Ok(Docker::connect_with_http(
            host,
            SOCKET_TIMEOUT_SECS,
            API_DEFAULT_VERSION,
        )?)
    } else {
        // assume bare path → unix
        Ok(Docker::connect_with_socket(
            host,
            SOCKET_TIMEOUT_SECS,
            API_DEFAULT_VERSION,
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    struct FakeEnv {
        host_flag: Option<String>,
        docker_host: Option<String>,
        home: Option<PathBuf>,
        paths: RefCell<HashMap<String, bool>>,
        files: RefCell<HashMap<PathBuf, String>>,
        dirs: RefCell<HashMap<PathBuf, Vec<PathBuf>>>,
    }

    impl SocketEnv for FakeEnv {
        fn host_flag(&self) -> Option<String> {
            self.host_flag.clone()
        }
        fn env_docker_host(&self) -> Option<String> {
            self.docker_host.clone()
        }
        fn home(&self) -> Option<PathBuf> {
            self.home.clone()
        }
        fn path_exists(&self, p: &str) -> bool {
            *self.paths.borrow().get(p).unwrap_or(&false)
        }
        fn read_to_string(&self, p: &Path) -> Option<String> {
            self.files.borrow().get(p).cloned()
        }
        fn list_dir(&self, p: &Path) -> Vec<PathBuf> {
            self.dirs.borrow().get(p).cloned().unwrap_or_default()
        }
    }

    fn blank_env() -> FakeEnv {
        FakeEnv {
            host_flag: None,
            docker_host: None,
            home: Some(PathBuf::from("/home/u")),
            paths: RefCell::new(HashMap::new()),
            files: RefCell::new(HashMap::new()),
            dirs: RefCell::new(HashMap::new()),
        }
    }

    #[test]
    fn cli_flag_wins_over_everything() {
        let mut env = blank_env();
        env.host_flag = Some("unix:///custom.sock".into());
        env.docker_host = Some("tcp://ignored".into());
        let got = resolve_socket(&env).unwrap();
        assert!(matches!(got, SocketSource::CliFlag(ref s) if s == "unix:///custom.sock"));
    }

    #[test]
    fn env_var_used_when_no_flag() {
        let mut env = blank_env();
        env.docker_host = Some("tcp://1.2.3.4:2375".into());
        let got = resolve_socket(&env).unwrap();
        assert!(matches!(got, SocketSource::EnvVar(ref s) if s == "tcp://1.2.3.4:2375"));
    }

    #[test]
    fn falls_back_to_colima_default() {
        let env = blank_env();
        let colima_path = "/home/u/.colima/default/docker.sock";
        env.paths.borrow_mut().insert(colima_path.into(), true);
        let got = resolve_socket(&env).unwrap();
        assert!(
            matches!(got, SocketSource::ColimaDefault(ref s) if s == &format!("unix://{}", colima_path))
        );
    }

    #[test]
    fn falls_back_to_system_socket_last() {
        let env = blank_env();
        env.paths
            .borrow_mut()
            .insert("/var/run/docker.sock".into(), true);
        let got = resolve_socket(&env).unwrap();
        assert!(
            matches!(got, SocketSource::SystemDefault(ref s) if s == "unix:///var/run/docker.sock")
        );
    }

    #[test]
    fn errors_when_no_socket_available() {
        let env = blank_env();
        assert!(resolve_socket(&env).is_err());
    }

    #[test]
    fn docker_context_parsed_from_meta_json() {
        let env = blank_env();
        let meta_dir = PathBuf::from("/home/u/.docker/contexts/meta");
        let ctx_dir = meta_dir.join("abc123");
        let meta_file = ctx_dir.join("meta.json");
        env.dirs.borrow_mut().insert(meta_dir, vec![ctx_dir]);
        env.files.borrow_mut().insert(
            meta_file,
            r#"{"Name":"colima","Endpoints":{"docker":{"Host":"unix:///tmp/colima.sock"}}}"#.into(),
        );
        let got = resolve_socket(&env).unwrap();
        assert!(
            matches!(got, SocketSource::DockerContext(ref s) if s == "unix:///tmp/colima.sock")
        );
    }

    #[test]
    fn bad_meta_json_falls_through() {
        let env = blank_env();
        let meta_dir = PathBuf::from("/home/u/.docker/contexts/meta");
        let ctx_dir = meta_dir.join("broken");
        let meta_file = ctx_dir.join("meta.json");
        env.dirs.borrow_mut().insert(meta_dir, vec![ctx_dir]);
        env.files.borrow_mut().insert(meta_file, "not json".into());
        // colima fallback present
        env.paths
            .borrow_mut()
            .insert("/home/u/.colima/default/docker.sock".into(), true);
        let got = resolve_socket(&env).unwrap();
        assert!(matches!(got, SocketSource::ColimaDefault(_)));
    }
}
