use anyhow::{anyhow, Result};
use std::process::Command;
use std::time::Duration;

pub struct SingboxProcess {
    pub binary_path: String,
    pub config_path: String,
}

impl SingboxProcess {
    pub fn new(bin: &str, cfg: &str) -> Self {
        Self {
            binary_path: bin.into(),
            config_path: cfg.into(),
        }
    }
    pub fn is_running(&self) -> Option<bool> {
        Command::new("pgrep")
            .args(["-x", "sing-box"])
            .output()
            .ok()
            .map(|o| o.status.success())
    }
    pub fn start(&self) -> Result<()> {
        systemctl("start")
    }
    pub fn stop(&self) -> Result<()> {
        systemctl("stop")
    }
    pub fn reload(&self) -> Result<()> {
        systemctl("reload").or_else(|_| systemctl("restart"))
    }
    pub fn check_config(&self) -> Result<()> {
        let o = Command::new(&self.binary_path)
            .args(["check", "-c", &self.config_path])
            .output()?;
        if o.status.success() {
            Ok(())
        } else {
            Err(anyhow!(
                "配置验证失败: {}",
                String::from_utf8_lossy(&o.stderr)
            ))
        }
    }
}

fn systemctl(action: &str) -> Result<()> {
    let status = Command::new("systemctl")
        .args([action, "sing-box"])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("systemctl {} sing-box 失败", action))
    }
}

/// 内核管理：不依赖 SingboxProcess 实例（卸载时二进制可能不存在）。
#[derive(Debug, Clone)]
pub struct KernelStatus {
    pub installed: bool,
    pub running: Option<bool>,
    pub enabled: bool,
    pub version: Option<String>,
    pub binary_path: Option<String>,
}

pub fn status() -> KernelStatus {
    let binary_path = locate_binary();
    let version = binary_path.as_deref().and_then(read_version);
    KernelStatus {
        installed: binary_path.is_some(),
        running: detect_running(),
        enabled: is_enabled(),
        version,
        binary_path,
    }
}

fn locate_binary() -> Option<String> {
    let p = "/etc/sing-box/bin/sing-box";
    if std::path::Path::new(p).exists() {
        return Some(p.into());
    }
    Command::new("sh")
        .args(["-c", "command -v sing-box"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if s.is_empty() {
                    None
                } else {
                    Some(s)
                }
            } else {
                None
            }
        })
}

fn read_version(bin: &str) -> Option<String> {
    let o = Command::new(bin).arg("version").output().ok()?;
    if !o.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&o.stdout);
    // 第一行一般形如 "sing-box version 1.10.0"
    s.lines().next().map(|l| l.trim().to_string())
}

fn detect_running() -> Option<bool> {
    Command::new("pgrep")
        .args(["-x", "sing-box"])
        .output()
        .ok()
        .map(|o| o.status.success())
}

fn is_enabled() -> bool {
    Command::new("systemctl")
        .args(["is-enabled", "sing-box"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn enable() -> Result<()> {
    systemctl("enable")
}
pub fn disable() -> Result<()> {
    systemctl("disable")
}
pub fn start() -> Result<()> {
    systemctl("start")
}
pub fn stop() -> Result<()> {
    systemctl("stop")
}
pub fn restart() -> Result<()> {
    systemctl("restart")
}

/// 从 Github 获取官方版 sing-box 二进制并安装到 /etc/sing-box/bin
pub async fn install_latest() -> Result<()> {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "linux-amd64",
        "aarch64" => "linux-arm64",
        other => return Err(anyhow::anyhow!("暂不支持的架构: {}", other)),
    };

    let client = reqwest::Client::builder()
        .user_agent("sb-manager")
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let url = "https://api.github.com/repos/SagerNet/sing-box/releases/latest";
    let body = client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let release: serde_json::Value = serde_json::from_str(&body)?;

    let tag = release["tag_name"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("无法获取最新 tag"))?;
    let version = tag.trim_start_matches('v');
    let asset = format!("sing-box-{}-{}.tar.gz", version, arch);
    let download_url = format!(
        "https://github.com/SagerNet/sing-box/releases/download/{}/{}",
        tag, asset
    );

    let tmp_dir = std::env::temp_dir().join(format!("sbm-singbox-{}", version));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir)?;

    let tarball = tmp_dir.join(&asset);
    let bytes = client
        .get(&download_url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    std::fs::write(&tarball, &bytes)?;

    let status = Command::new("tar")
        .args(["xzf"])
        .arg(&tarball)
        .arg("-C")
        .arg(&tmp_dir)
        .status()?;
    if !status.success() {
        return Err(anyhow::anyhow!("解压 tarball 失败"));
    }

    let inner = tmp_dir.join(format!("sing-box-{}-{}", version, arch));
    let src_bin = inner.join("sing-box");
    if !src_bin.exists() {
        return Err(anyhow::anyhow!("tarball 内未找到 sing-box 二进制"));
    }

    let _ = Command::new("systemctl")
        .args(["stop", "sing-box"])
        .status();
    std::fs::create_dir_all("/etc/sing-box/bin")?;
    let dst = std::path::Path::new("/etc/sing-box/bin/sing-box");
    std::fs::copy(&src_bin, dst)?;
    set_executable(dst)?;

    let unit_path = std::path::Path::new("/etc/systemd/system/sing-box.service");
    std::fs::create_dir_all("/etc/systemd/system")?;
    std::fs::write(unit_path, SINGBOX_UNIT)?;
    let _ = Command::new("systemctl").arg("daemon-reload").status();

    let cfg_dir = std::path::Path::new("/etc/sing-box");
    let cfg_file = cfg_dir.join("config.json");
    if !cfg_file.exists() {
        std::fs::write(&cfg_file, DEFAULT_CONFIG_WITH_V2RAY_API)?;
    }

    let _ = Command::new("systemctl")
        .args(["enable", "--now", "sing-box"])
        .status();

    let _ = std::fs::remove_dir_all(&tmp_dir);
    Ok(())
}

/// 卸载：停服务、禁用、删二进制 / unit 文件、daemon-reload。
pub fn uninstall() -> Result<()> {
    let _ = Command::new("systemctl")
        .args(["stop", "sing-box"])
        .status();
    let _ = Command::new("systemctl")
        .args(["disable", "sing-box"])
        .status();
    for p in [
        "/etc/sing-box/bin/sing-box",
        "/usr/local/bin/sing-box",
        "/etc/systemd/system/sing-box.service",
    ] {
        let _ = std::fs::remove_file(p);
    }
    let _ = Command::new("systemctl").arg("daemon-reload").status();
    Ok(())
}

/// 内嵌的 systemd 单元模板
const SINGBOX_UNIT: &str = r#"[Unit]
Description=sing-box service
Documentation=https://sing-box.sagernet.org
After=network.target nss-lookup.target network-online.target

[Service]
CapabilityBoundingSet=CAP_NET_ADMIN CAP_NET_BIND_SERVICE CAP_NET_RAW CAP_SYS_PTRACE CAP_DAC_READ_SEARCH
AmbientCapabilities=CAP_NET_ADMIN CAP_NET_BIND_SERVICE CAP_NET_RAW CAP_SYS_PTRACE CAP_DAC_READ_SEARCH
ExecStart=/etc/sing-box/bin/sing-box -D /var/lib/sing-box -C /etc/sing-box run
ExecReload=/bin/kill -HUP $MAINPID
Restart=on-failure
RestartSec=10s
LimitNOFILE=infinity

[Install]
WantedBy=multi-user.target
"#;

/// 安装 with_v2ray_api 的自编译 sing-box。
/// 从 `repo` 仓库 releases 中挑最新 `singbox-v*` tag，下载匹配当前架构的 tarball。
pub async fn install_v2rayapi(repo: &str) -> Result<()> {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "linux-amd64",
        "aarch64" => "linux-arm64",
        other => return Err(anyhow!("暂不支持的架构: {}", other)),
    };

    let client = reqwest::Client::builder()
        .user_agent("sb-manager")
        .timeout(Duration::from_secs(30))
        .build()?;

    // 取最新 singbox-* release
    let url = format!("https://api.github.com/repos/{}/releases?per_page=30", repo);
    let body = client
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let releases: serde_json::Value = serde_json::from_str(&body)?;
    let tag = releases
        .as_array()
        .and_then(|arr| {
            arr.iter().find_map(|r| {
                let t = r["tag_name"].as_str()?;
                if t.starts_with("singbox-") && !r["prerelease"].as_bool().unwrap_or(false) {
                    Some(t.to_string())
                } else {
                    None
                }
            })
        })
        .ok_or_else(|| {
            anyhow!(
                "仓库 {} 里找不到 singbox-* release，请先跑一次 build-singbox workflow",
                repo
            )
        })?;

    let version = tag.trim_start_matches("singbox-");
    let asset = format!("sing-box-{}-{}-v2rayapi.tar.gz", version, arch);
    let download_url = format!(
        "https://github.com/{}/releases/download/{}/{}",
        repo, tag, asset
    );

    let tmp_dir = std::env::temp_dir().join(format!("sbm-singbox-{}", version));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir)?;

    let tarball = tmp_dir.join(&asset);
    let bytes = client
        .get(&download_url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    std::fs::write(&tarball, &bytes)?;

    // 尽量校验 sha256
    let sha_url = format!("{}.sha256", download_url);
    if let Ok(resp) = client.get(&sha_url).send().await {
        if let Ok(text) = resp.text().await {
            verify_sha256(&tarball, text.trim())?;
        }
    }

    // 解包
    let status = Command::new("tar")
        .args(["xzf"])
        .arg(&tarball)
        .arg("-C")
        .arg(&tmp_dir)
        .status()?;
    if !status.success() {
        return Err(anyhow!("解压 tarball 失败"));
    }

    let inner = tmp_dir.join(format!("sing-box-{}-{}-v2rayapi", version, arch));
    let src_bin = inner.join("sing-box");
    if !src_bin.exists() {
        return Err(anyhow!("tarball 内未找到 sing-box 二进制"));
    }

    // 停服务 → 替换 → 启服务
    let _ = Command::new("systemctl")
        .args(["stop", "sing-box"])
        .status();
    std::fs::create_dir_all("/etc/sing-box/bin")?;
    let dst = std::path::Path::new("/etc/sing-box/bin/sing-box");
    std::fs::copy(&src_bin, dst)?;
    set_executable(dst)?;

    let unit_path = std::path::Path::new("/etc/systemd/system/sing-box.service");
    std::fs::create_dir_all("/etc/systemd/system")?;
    std::fs::write(unit_path, SINGBOX_UNIT)?;
    let _ = Command::new("systemctl").arg("daemon-reload").status();

    // 确保 /etc/sing-box/config.json 存在（用最小骨架）
    let cfg_dir = std::path::Path::new("/etc/sing-box");
    if !cfg_dir.exists() {
        std::fs::create_dir_all(cfg_dir)?;
    }
    let cfg_file = cfg_dir.join("config.json");
    if !cfg_file.exists() {
        std::fs::write(&cfg_file, DEFAULT_CONFIG_WITH_V2RAY_API)?;
    }

    // 自动设为开机自启 + 启动（失败不致命，用户可在内核页重试）
    let _ = Command::new("systemctl")
        .args(["enable", "sing-box"])
        .status();
    let _ = Command::new("systemctl")
        .args(["restart", "sing-box"])
        .status();

    // 清理临时目录（忽略失败）
    let _ = std::fs::remove_dir_all(&tmp_dir);
    Ok(())
}

fn verify_sha256(file: &std::path::Path, sha_line: &str) -> Result<()> {
    let expected = sha_line
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if expected.len() != 64 {
        return Ok(());
    } // 非法 sha 文件，跳过校验
    let out = Command::new("sha256sum").arg(file).output()?;
    if !out.status.success() {
        return Ok(());
    }
    let got = String::from_utf8_lossy(&out.stdout);
    let got = got
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if got != expected {
        return Err(anyhow!(
            "sha256 校验失败: expected {} got {}",
            expected,
            got
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn set_executable(path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perm = std::fs::metadata(path)?.permissions();
    perm.set_mode(0o755);
    std::fs::set_permissions(path, perm)?;
    Ok(())
}
#[cfg(not(unix))]
fn set_executable(_: &std::path::Path) -> Result<()> {
    Ok(())
}

const DEFAULT_CONFIG_WITH_V2RAY_API: &str = r#"{
  "log": { "level": "info", "timestamp": true },
  "inbounds": [],
  "outbounds": [
    { "type": "direct", "tag": "direct" },
    { "type": "block",  "tag": "block"  }
  ],
  "experimental": {
    "v2ray_api": {
      "listen": "127.0.0.1:18080",
      "stats": { "enabled": true, "users": [] }
    }
  }
}
"#;
