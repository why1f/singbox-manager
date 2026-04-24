use std::path::Path;

use anyhow::Result;
use sqlx::SqlitePool;

use crate::{core, model::config::AppConfig, service::user_service};

pub async fn apply_user_runtime_changes(pool: &SqlitePool, cfg: &AppConfig) -> Result<()> {
    if !Path::new(&cfg.singbox.config_path).exists() {
        return Ok(());
    }
    let mut config = core::config::load(&cfg.singbox.config_path)?;
    let users = user_service::list_users(pool).await?;
    core::config::sync_users(&mut config, &users, &cfg.singbox.grpc_addr);
    core::config::save(&cfg.singbox.config_path, &config)?;
    let proc =
        core::singbox::SingboxProcess::new(&cfg.singbox.binary_path, &cfg.singbox.config_path);
    proc.check_config()?;
    if matches!(proc.is_running(), Some(true)) {
        proc.reload()?;
    }
    Ok(())
}
