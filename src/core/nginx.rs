use anyhow::{anyhow, Context, Result};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct NginxStatus {
    pub installed: bool,
    pub running: Option<bool>,
    pub enabled: bool,
    pub version: Option<String>,
    pub binary_path: Option<String>,
    pub conf_exists: bool,
}

pub fn status(conf_path: &str) -> NginxStatus {
    let binary_path = locate_binary();
    let version = binary_path.as_deref().and_then(read_version);
    NginxStatus {
        installed:   binary_path.is_some(),
        running:     detect_running(),
        enabled:     is_enabled(),
        version,
        binary_path,
        conf_exists: Path::new(conf_path).exists(),
    }
}

fn locate_binary() -> Option<String> {
    for p in ["/usr/sbin/nginx", "/usr/local/sbin/nginx", "/usr/bin/nginx"] {
        if Path::new(p).exists() { return Some(p.into()); }
    }
    Command::new("sh").args(["-c", "command -v nginx"]).output().ok()
        .and_then(|o| if o.status.success() {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if s.is_empty() { None } else { Some(s) }
        } else { None })
}

fn read_version(bin: &str) -> Option<String> {
    let o = Command::new(bin).arg("-v").output().ok()?;
    // nginx -v prints to stderr
    let s = String::from_utf8_lossy(if o.stdout.is_empty() { &o.stderr } else { &o.stdout });
    s.lines().next().map(|l| l.trim().to_string())
}

fn detect_running() -> Option<bool> {
    Command::new("pgrep").args(["-x", "nginx"])
        .output().ok().map(|o| o.status.success())
}

fn is_enabled() -> bool {
    Command::new("systemctl").args(["is-enabled", "nginx"])
        .output().map(|o| o.status.success()).unwrap_or(false)
}

fn systemctl(action: &str) -> Result<()> {
    let status = Command::new("systemctl").args([action, "nginx"]).status()?;
    if status.success() { Ok(()) } else { Err(anyhow!("systemctl {} nginx 失败", action)) }
}

pub fn enable()  -> Result<()> { systemctl("enable") }
pub fn disable() -> Result<()> { systemctl("disable") }
pub fn start()   -> Result<()> { systemctl("start") }
pub fn stop()    -> Result<()> { systemctl("stop") }
pub fn restart() -> Result<()> { systemctl("restart") }
pub fn reload()  -> Result<()> { systemctl("reload") }

pub fn install_via_pkg() -> Result<()> {
    // 尝试多种包管理器
    if which("apt-get") {
        run_cmd("sh", &["-c", "DEBIAN_FRONTEND=noninteractive apt-get update && apt-get install -y nginx"])
    } else if which("dnf") {
        run_cmd("dnf", &["install", "-y", "nginx"])
    } else if which("yum") {
        run_cmd("yum", &["install", "-y", "nginx"])
    } else if which("pacman") {
        run_cmd("pacman", &["-Sy", "--noconfirm", "nginx"])
    } else if which("apk") {
        run_cmd("apk", &["add", "--no-cache", "nginx"])
    } else {
        Err(anyhow!("未识别的包管理器，请手动安装 nginx"))
    }
}

pub fn test_config() -> Result<String> {
    let o = Command::new("nginx").arg("-t").output()
        .context("调用 nginx -t 失败（nginx 是否已安装？）")?;
    let combined = format!("{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr));
    if o.status.success() { Ok(combined) }
    else { Err(anyhow!("nginx -t 校验失败:\n{}", combined)) }
}

fn which(bin: &str) -> bool {
    Command::new("sh").args(["-c", &format!("command -v {}", bin)]).status()
        .map(|s| s.success()).unwrap_or(false)
}
fn run_cmd(prog: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(prog).args(args).status()?;
    if status.success() { Ok(()) } else { Err(anyhow!("{} {:?} 返回非零", prog, args)) }
}

/// 生成 nginx 反代配置（写到 conf_path）。
/// public_base 形如 https://sub.example.com，从中解析出 server_name。
pub fn generate_conf(conf_path: &str, public_base: &str, upstream_listen: &str) -> Result<()> {
    // 从 public_base 抽 server_name
    let host = public_base
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/').next().unwrap_or(public_base);
    if host.is_empty() {
        return Err(anyhow!("public_base 为空，先在 config.toml 填上 [subscription].public_base"));
    }

    let template = format!(
r#"# 由 sb-manager 生成；可按需手改，下次按 [g] 会覆盖
server {{
    listen 443 ssl http2;
    listen [::]:443 ssl http2;
    server_name {host};

    # ▼ 这两行请改为你实际的证书路径（acme.sh 或其他方式）
    ssl_certificate     /etc/nginx/certs/{host}/fullchain.pem;
    ssl_certificate_key /etc/nginx/certs/{host}/privkey.pem;
    ssl_protocols TLSv1.2 TLSv1.3;

    # /sub/<token> 反代到 sb-manager；token 只允许 16-64 的 URL-safe 字符
    location ~ ^/sub/[A-Za-z0-9_-]{{16,64}}$ {{
        proxy_pass              http://{upstream};
        proxy_http_version      1.1;
        proxy_set_header Host            $host;
        proxy_set_header X-Real-IP       $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_pass_header       subscription-userinfo;
    }}

    # 其他路径 404，避免暴露管理端
    location / {{ return 404; }}
}}

server {{
    listen 80;
    listen [::]:80;
    server_name {host};
    return 301 https://$host$request_uri;
}}
"#,
        host = host, upstream = upstream_listen,
    );

    if let Some(p) = Path::new(conf_path).parent() {
        std::fs::create_dir_all(p).with_context(|| format!("创建 nginx 配置目录 {} 失败", p.display()))?;
    }
    std::fs::write(conf_path, template)
        .with_context(|| format!("写入 {} 失败", conf_path))?;
    Ok(())
}
