use std::path::Path;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use sqlx::{pool::PoolConnection, Sqlite, SqlitePool};
use tracing::warn;

use crate::{
    core::{
        config::{self, MetaOp},
        grpc::{query_all_traffic, StatsClient},
        singbox::{check_config_at, SingboxProcess},
        traffic::calc_deltas,
    },
    model::{config::AppConfig, traffic::TrafficDelta, user::User},
};

/// 跨进程写锁：用 SQLite `BEGIN IMMEDIATE` 把 CLI / daemon / TUI 三类进程对
/// config.json 的"读 → 改 → 写"串行化。
///
/// 未显式 commit/rollback 就被 drop（future 被取消、panic 等）时，Drop 里同步
/// 发一次 ROLLBACK 兜底——否则带着未结事务的连接会回到池里，后续复用者会在
/// 残留事务中执行语句，或长期占着 IMMEDIATE 锁把别的进程堵死。
pub struct RuntimeLock {
    conn: Option<PoolConnection<Sqlite>>,
}

impl RuntimeLock {
    pub async fn acquire(pool: &SqlitePool) -> Result<Self> {
        let mut conn = pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;
        Ok(Self { conn: Some(conn) })
    }

    pub fn conn(&mut self) -> &mut PoolConnection<Sqlite> {
        self.conn
            .as_mut()
            .expect("RuntimeLock 在 commit/rollback 之后不应再被使用")
    }

    pub async fn commit(mut self) -> Result<()> {
        if let Some(mut conn) = self.conn.take() {
            sqlx::query("COMMIT").execute(&mut *conn).await?;
        }
        Ok(())
    }

    pub async fn rollback(mut self) {
        if let Some(mut conn) = self.conn.take() {
            let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
        }
    }
}

impl Drop for RuntimeLock {
    fn drop(&mut self) {
        // 仍持有连接 = 既没 commit 也没 rollback，说明是取消/panic 路径。
        // 这里只能同步收尾：sqlx 的 SqliteConnection 在 drop 时会关闭底层连接，
        // 未提交事务由 SQLite 自动回滚；显式丢弃可以让它尽快发生。
        if let Some(conn) = self.conn.take() {
            warn!("RuntimeLock 未显式结束事务即被丢弃（任务取消或 panic），事务将回滚");
            drop(conn);
        }
    }
}

pub async fn mutate_config_locked<T, F>(
    pool: &SqlitePool,
    config_path: &str,
    binary_path: Option<&str>,
    create_if_missing: bool,
    mutate: F,
) -> Result<T>
where
    F: FnOnce(&mut Value, &mut Vec<MetaOp>) -> Result<T>,
{
    let lock = RuntimeLock::acquire(pool).await?;
    let mut meta_ops: Vec<MetaOp> = Vec::new();
    let result = (|| -> Result<T> {
        let mut config_json = if Path::new(config_path).exists() {
            config::load(config_path)?
        } else if create_if_missing {
            json!({ "inbounds": [], "outbounds": [] })
        } else {
            return Err(anyhow!("config.json 不存在"));
        };
        let out = mutate(&mut config_json, &mut meta_ops)?;
        save_with_optional_validate(config_path, binary_path, &config_json)?;
        Ok(out)
    })();
    match result {
        Ok(out) => {
            lock.commit().await?;
            // 配置文件已原子覆盖，meta 副作用此时才落盘——若 meta 写失败仅 warn，
            // 因为 config.json 是 source of truth；reality 节点丢失 public_key
            // 的极小概率（fs 写错误）下，下次 add 同 tag 会重生成。
            config::apply_meta_ops(&meta_ops);
            Ok(out)
        }
        Err(e) => {
            lock.rollback().await;
            Err(e)
        }
    }
}

/// save 到 `<path>.tmp`，可选用 sing-box 校验 .tmp，通过后原子 rename 到主路径。
/// 任一步失败都会清理 .tmp 并把错误向上抛，**主路径绝不会被坏配置覆盖**。
fn save_with_optional_validate(
    config_path: &str,
    binary_path: Option<&str>,
    json: &Value,
) -> Result<()> {
    let tmp = format!("{}.tmp", config_path);
    if let Some(parent) = Path::new(config_path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).ok();
        }
    }
    std::fs::write(&tmp, serde_json::to_string_pretty(json)?)?;
    if let Some(bin) = binary_path {
        if Path::new(bin).exists() {
            if let Err(e) = check_config_at(bin, Path::new(&tmp)) {
                let _ = std::fs::remove_file(&tmp);
                return Err(e);
            }
        }
    }
    std::fs::rename(&tmp, config_path)?;
    Ok(())
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
        let mut config_json = config::load(&cfg.singbox.config_path)?;
        let users = list_all_users(lock.conn()).await?;
        config::sync_users(&mut config_json, &users, &cfg.singbox.grpc_addr);
        // sync_users 不动 meta；走同样的"save .tmp + validate + rename"流程，
        // 防止坏配置（理论上不会发生，但保险起见）覆盖主路径。
        save_with_optional_validate(
            &cfg.singbox.config_path,
            Some(&cfg.singbox.binary_path),
            &config_json,
        )?;
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
    // 此处主路径已经过 check + rename，validate_and_reload 内部会再 check 一次（冗余但稳妥）
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
