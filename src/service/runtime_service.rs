use std::path::Path;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use sqlx::{pool::PoolConnection, Sqlite, SqlitePool};
use tracing::warn;

use crate::{
    core::{
        config,
        grpc::{query_all_traffic, StatsClient},
        singbox::SingboxProcess,
        traffic::calc_deltas,
    },
    model::{config::AppConfig, traffic::TrafficDelta, user::User},
};

pub struct RuntimeLock {
    conn: PoolConnection<Sqlite>,
    finished: bool,
}

impl RuntimeLock {
    pub async fn acquire(pool: &SqlitePool) -> Result<Self> {
        let mut conn = pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;
        Ok(Self {
            conn,
            finished: false,
        })
    }

    pub fn conn(&mut self) -> &mut PoolConnection<Sqlite> {
        &mut self.conn
    }

    pub async fn commit(mut self) -> Result<()> {
        if !self.finished {
            sqlx::query("COMMIT").execute(&mut *self.conn).await?;
            self.finished = true;
        }
        Ok(())
    }

    pub async fn rollback(mut self) {
        if !self.finished {
            let _ = sqlx::query("ROLLBACK").execute(&mut *self.conn).await;
            self.finished = true;
        }
    }
}

pub async fn mutate_config_locked<T, F>(
    pool: &SqlitePool,
    config_path: &str,
    create_if_missing: bool,
    mutate: F,
) -> Result<T>
where
    F: FnOnce(&mut Value) -> Result<T>,
{
    let lock = RuntimeLock::acquire(pool).await?;
    let result = (|| -> Result<T> {
        let mut config = if Path::new(config_path).exists() {
            config::load(config_path)?
        } else if create_if_missing {
            json!({ "inbounds": [], "outbounds": [] })
        } else {
            return Err(anyhow!("config.json 不存在"));
        };
        let out = mutate(&mut config)?;
        config::save(config_path, &config)?;
        Ok(out)
    })();
    match result {
        Ok(out) => {
            lock.commit().await?;
            Ok(out)
        }
        Err(e) => {
            lock.rollback().await;
            Err(e)
        }
    }
}

pub async fn validate_and_reload(pool: &SqlitePool, cfg: &AppConfig) -> Result<()> {
    let proc = SingboxProcess::new(&cfg.singbox.binary_path, &cfg.singbox.config_path);
    proc.check_config()?;
    if matches!(proc.is_running(), Some(true)) {
        if let Err(e) = flush_current_traffic(pool, &cfg.singbox.grpc_addr).await {
            warn!("reload 前预同步流量失败: {}", e);
        }
        proc.reload()?;
    }
    Ok(())
}

pub async fn apply_user_runtime_changes(pool: &SqlitePool, cfg: &AppConfig) -> Result<()> {
    if !Path::new(&cfg.singbox.config_path).exists() {
        return Ok(());
    }
    let mut lock = RuntimeLock::acquire(pool).await?;
    let result = async {
        let mut config = config::load(&cfg.singbox.config_path)?;
        let users = list_all_users(lock.conn()).await?;
        config::sync_users(&mut config, &users, &cfg.singbox.grpc_addr);
        config::save(&cfg.singbox.config_path, &config)?;
        Ok::<(), anyhow::Error>(())
    }
    .await;
    match result {
        Ok(()) => lock.commit().await?,
        Err(e) => {
            lock.rollback().await;
            return Err(e);
        }
    }
    validate_and_reload(pool, cfg).await
}

pub async fn flush_current_traffic(
    pool: &SqlitePool,
    grpc_addr: &str,
) -> Result<Vec<TrafficDelta>> {
    let mut grpc = crate::core::grpc::connect(grpc_addr).await?;
    sync_current_traffic(pool, &mut grpc).await
}

pub async fn sync_current_traffic_with_users(
    pool: &SqlitePool,
    grpc: &mut StatsClient,
) -> Result<(Vec<User>, Vec<TrafficDelta>)> {
    let snaps = query_all_traffic(grpc, false).await?;
    let mut lock = RuntimeLock::acquire(pool).await?;
    let result = sync_current_traffic_with_users_locked(lock.conn(), &snaps).await;
    match result {
        Ok(out) => {
            lock.commit().await?;
            Ok(out)
        }
        Err(e) => {
            lock.rollback().await;
            Err(e)
        }
    }
}

async fn sync_current_traffic(
    pool: &SqlitePool,
    grpc: &mut StatsClient,
) -> Result<Vec<TrafficDelta>> {
    let (_, deltas) = sync_current_traffic_with_users(pool, grpc).await?;
    Ok(deltas)
}

async fn sync_current_traffic_with_users_locked(
    conn: &mut PoolConnection<Sqlite>,
    snaps: &[crate::model::traffic::LiveTrafficSnapshot],
) -> Result<(Vec<User>, Vec<TrafficDelta>)> {
    let users = list_all_users(conn).await?;
    let deltas = calc_deltas(snaps, &users);
    if !deltas.is_empty() {
        for d in &deltas {
            sqlx::query(
                r#"UPDATE users SET used_up_bytes=used_up_bytes+?,
                used_down_bytes=used_down_bytes+?,last_live_up=?,last_live_down=? WHERE name=?"#,
            )
            .bind(d.delta_up)
            .bind(d.delta_down)
            .bind(d.new_live_up)
            .bind(d.new_live_down)
            .bind(&d.username)
            .execute(&mut **conn)
            .await?;
            sqlx::query("INSERT INTO traffic_history(username,up_bytes,down_bytes,recorded_at)VALUES(?,?,?,datetime('now'))")
                .bind(&d.username)
                .bind(d.delta_up)
                .bind(d.delta_down)
                .execute(&mut **conn)
                .await?;
        }
    }
    Ok((users, deltas))
}

async fn list_all_users(conn: &mut PoolConnection<Sqlite>) -> Result<Vec<User>> {
    Ok(
        sqlx::query_as::<_, User>("SELECT * FROM users ORDER BY name")
            .fetch_all(&mut **conn)
            .await?,
    )
}
