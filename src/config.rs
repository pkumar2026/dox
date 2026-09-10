use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub refresh_ms: u64,
    pub log_buffer_lines: usize,
    pub log_tail_initial: u64,
    pub host: String,
    pub mouse: bool,
    pub confirm_destructive: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            refresh_ms: 1000,
            log_buffer_lines: 10_000,
            log_tail_initial: 500,
            host: "auto".into(),
            // On by default. Wheel scrolls logs; click-drag selects log lines
            // in-app; `y` copies. For native terminal selection, hold Shift
            // while dragging (alacritty/ghostty/kitty all support this).
            mouse: true,
            confirm_destructive: true,
        }
    }
}

pub fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|p| p.join("dox").join("config.toml"))
}

pub fn load() -> Result<Config> {
    let Some(path) = config_path() else {
        return Ok(Config::default());
    };
    if !path.exists() {
        return Ok(Config::default());
    }
    let raw = std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let cfg: Config = toml::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let cfg = Config::default();
        assert_eq!(cfg.refresh_ms, 1000);
        assert_eq!(cfg.log_buffer_lines, 10_000);
        assert!(cfg.confirm_destructive);
    }

    #[test]
    fn partial_toml_uses_defaults_for_missing_keys() {
        let raw = r#"refresh_ms = 500"#;
        let cfg: Config = toml::from_str(raw).unwrap();
        assert_eq!(cfg.refresh_ms, 500);
        assert_eq!(cfg.log_buffer_lines, 10_000);
    }
}
