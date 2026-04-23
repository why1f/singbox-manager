use anyhow::{anyhow, Result};
use std::process::Command;
use std::path::{Path, PathBuf};

const BACKUP_DIR: &str = "/etc/sing-box/backup";

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
    let backup_path = Path::new(BACKUP_DIR).join(filename);
    if !backup_path.exists() {
        return Err(anyhow!("备份文件不存在: {}", backup_path.display()));
    }

    let status = Command::new("tar")
        .args(["xzf", backup_path.to_str().unwrap(), "-P"])
        .status()?;

    if !status.success() {
        return Err(anyhow!("解压备份文件失败"));
    }

    // 重启相关服务
    let _ = Command::new("systemctl").args(["restart", "singbox-manager"]).status();
    let _ = Command::new("systemctl").args(["restart", "sing-box"]).status();
    let _ = Command::new("systemctl").args(["reload", "nginx"]).status();

    Ok(())
}
