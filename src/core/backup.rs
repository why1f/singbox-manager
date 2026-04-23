use anyhow::{anyhow, Result};
use std::process::Command;
use std::path::{Path, PathBuf};

const BACKUP_DIR: &str = "/etc/sing-box/backup";
const ALLOWLIST_DIRS: &[&str] = &[
    "/etc/sing-box/manager",
    "/etc/sing-box/certs",
];
const ALLOWLIST_FILES: &[&str] = &[
    "/etc/sing-box/config.json",
    "/etc/nginx/conf.d/sb-manager.conf",
];

pub fn create_backup() -> Result<String> {
    std::fs::create_dir_all(BACKUP_DIR)?;

    let now = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let backup_file = format!("{}/backup_{}.tar.gz", BACKUP_DIR, now);

    let paths = vec![
        "/etc/sing-box/manager",
        "/etc/sing-box/certs",
        "/etc/sing-box/config.json",
        "/etc/nginx/conf.d/sb-manager.conf",
    ];

    let mut args = vec!["czf".to_string(), backup_file.clone(), "-P".to_string()];
    for p in paths {
        if Path::new(p).exists() {
            args.push(p.to_string());
        }
    }

    let status = Command::new("tar").args(&args).status()?;
    if !status.success() {
        return Err(anyhow!("执行 tar 命令备份失败"));
    }

    rotate_backups()?;

    Ok(backup_file)
}

fn rotate_backups() -> Result<()> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(BACKUP_DIR)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.to_string_lossy().ends_with(".tar.gz"))
        .collect();

    entries.sort_by_key(|a| std::cmp::Reverse(a.metadata().and_then(|m| m.modified()).unwrap_or(std::time::SystemTime::UNIX_EPOCH)));

    for old_file in entries.into_iter().skip(5) {
        let _ = std::fs::remove_file(old_file);
    }

    Ok(())
}

pub fn list_backups() -> Result<Vec<String>> {
    if !Path::new(BACKUP_DIR).exists() { return Ok(vec![]); }
    let mut entries: Vec<PathBuf> = std::fs::read_dir(BACKUP_DIR)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.to_string_lossy().ends_with(".tar.gz"))
        .collect();
    
    entries.sort_by_key(|a| std::cmp::Reverse(a.metadata().and_then(|m| m.modified()).unwrap_or(std::time::SystemTime::UNIX_EPOCH)));
    
    Ok(entries.into_iter().map(|p| p.file_name().unwrap_or_default().to_string_lossy().into_owned()).collect())
}

pub fn restore_backup(filename: &str) -> Result<()> {
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return Err(anyhow!("非法备份文件名: {}", filename));
    }
    let backup_path = Path::new(BACKUP_DIR).join(filename);
    if !backup_path.exists() {
        return Err(anyhow!("备份文件不存在: {}", backup_path.display()));
    }

    validate_backup_contents(&backup_path)?;

    let status = Command::new("tar")
        .args(["xzf", backup_path.to_str().unwrap(), "-P"])
        .status()?;

    if !status.success() {
        return Err(anyhow!("解压备份文件失败"));
    }

    // 重启相关服务
    let _ = Command::new("systemctl").args(["restart", "sb-manager"]).status();
    let _ = Command::new("systemctl").args(["restart", "sing-box"]).status();
    let _ = Command::new("systemctl").args(["reload", "nginx"]).status();

    Ok(())
}

fn validate_backup_contents(backup_path: &Path) -> Result<()> {
    let out = Command::new("tar")
        .args(["tzf"])
        .arg(backup_path)
        .output()?;
    if !out.status.success() {
        return Err(anyhow!("读取备份清单失败"));
    }

    let listing = String::from_utf8_lossy(&out.stdout);
    for raw in listing.lines().map(str::trim).filter(|s| !s.is_empty()) {
        let p = normalize_path(raw).ok_or_else(|| anyhow!("备份内存在非法路径: {}", raw))?;
        if !is_allowed_path(&p) {
            return Err(anyhow!("备份内包含不允许恢复的路径: {}", p));
        }
    }
    Ok(())
}

fn normalize_path(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if !raw.starts_with('/') {
        return None;
    }
    let p = Path::new(raw);
    let mut parts = Vec::new();
    for c in p.components() {
        use std::path::Component;
        match c {
            Component::RootDir => {}
            Component::Normal(seg) => parts.push(seg.to_string_lossy().to_string()),
            _ => return None,
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(format!("/{}", parts.join("/")))
}

fn is_allowed_path(path: &str) -> bool {
    ALLOWLIST_FILES.contains(&path)
        || ALLOWLIST_DIRS.iter().any(|dir| path == *dir || path.starts_with(&format!("{}/", dir)))
}

#[cfg(test)]
mod tests {
    use super::{is_allowed_path, normalize_path};

    #[test]
    fn normalize_absolute_path() {
        assert_eq!(normalize_path("/etc/sing-box/manager/manager.db"), Some("/etc/sing-box/manager/manager.db".into()));
    }

    #[test]
    fn reject_relative_or_traversal() {
        assert_eq!(normalize_path("../etc/passwd"), None);
        assert_eq!(normalize_path("etc/passwd"), None);
        assert!(!is_allowed_path("/etc/passwd"));
    }

    #[test]
    fn allow_only_expected_paths() {
        assert!(is_allowed_path("/etc/sing-box/manager/config.toml"));
        assert!(is_allowed_path("/etc/sing-box/config.json"));
        assert!(!is_allowed_path("/etc/passwd"));
    }
}
