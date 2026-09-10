use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

pub trait Clipboard: Send {
    fn set(&mut self, text: &str) -> Result<()>;
}

pub struct SystemClipboard;

impl Clipboard for SystemClipboard {
    fn set(&mut self, text: &str) -> Result<()> {
        // macOS: prefer pbcopy. arboard goes through NSPasteboard which
        // can hang or silently no-op inside a raw-mode TUI on background threads.
        #[cfg(target_os = "macos")]
        match pbcopy(text) {
            Ok(()) => Ok(()),
            Err(e) => {
                tracing::warn!(error = %e, "pbcopy failed, trying arboard");
                arboard::Clipboard::new()
                    .and_then(|mut c| c.set_text(text.to_string()))
                    .map_err(|e| anyhow::anyhow!("arboard: {}", e))
            }
        }
        #[cfg(not(target_os = "macos"))]
        match arboard::Clipboard::new().and_then(|mut c| c.set_text(text.to_string())) {
            Ok(()) => Ok(()),
            Err(e) => {
                tracing::warn!(error = %e, "arboard failed, falling back to xclip");
                xclip(text)
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn xclip(text: &str) -> Result<()> {
    let mut child = Command::new("xclip")
        .arg("-selection")
        .arg("clipboard")
        .stdin(Stdio::piped())
        .spawn()
        .context("spawn xclip")?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(text.as_bytes()).context("xclip stdin")?;
    }
    let status = child.wait().context("await xclip")?;
    if !status.success() {
        anyhow::bail!("xclip exited with {}", status);
    }
    Ok(())
}

fn pbcopy(text: &str) -> Result<()> {
    let mut child = Command::new("pbcopy")
        .stdin(Stdio::piped())
        .spawn()
        .context("spawn pbcopy (is this macOS?)")?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(text.as_bytes())
            .context("write to pbcopy stdin")?;
    }
    let status = child.wait().context("await pbcopy")?;
    if !status.success() {
        anyhow::bail!("pbcopy exited with {}", status);
    }
    Ok(())
}

#[cfg(test)]
pub struct FakeClipboard {
    pub contents: Option<String>,
}

#[cfg(test)]
impl FakeClipboard {
    pub fn new() -> Self {
        Self { contents: None }
    }
}

#[cfg(test)]
impl Clipboard for FakeClipboard {
    fn set(&mut self, text: &str) -> Result<()> {
        self.contents = Some(text.to_string());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_clipboard_records_set_text() {
        let mut c = FakeClipboard::new();
        c.set("hello world").unwrap();
        assert_eq!(c.contents.as_deref(), Some("hello world"));
    }
}
