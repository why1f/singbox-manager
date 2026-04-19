use anyhow::{anyhow, Result};
use std::process::Command;

pub struct SingboxProcess { pub binary_path: String, pub config_path: String }

impl SingboxProcess {
    pub fn new(bin: &str, cfg: &str) -> Self {
        Self { binary_path: bin.into(), config_path: cfg.into() }
    }
    pub fn is_running(&self) -> Option<bool> {
        Command::new("pgrep").args(["-x", "sing-box"])
            .output().ok().map(|o| o.status.success())
    }
    pub fn start(&self) -> Result<()> { self.systemctl("start") }
    pub fn stop(&self) -> Result<()> { self.systemctl("stop") }
    pub fn reload(&self) -> Result<()> {
        self.systemctl("reload").or_else(|_| self.systemctl("restart"))
    }
    fn systemctl(&self, action: &str) -> Result<()> {
        let status = Command::new("systemctl").args([action, "sing-box"]).status()?;
        if status.success() { Ok(()) } else { Err(anyhow!("systemctl {} sing-box 失败", action)) }
    }
    pub fn check_config(&self) -> Result<()> {
        let o = Command::new(&self.binary_path).args(["check", "-c", &self.config_path]).output()?;
        if o.status.success() { Ok(()) }
        else { Err(anyhow!("配置验证失败: {}", String::from_utf8_lossy(&o.stderr))) }
    }
}
