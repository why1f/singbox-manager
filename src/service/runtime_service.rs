use std::path::Path;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use sqlx::{Sqlite, SqliteConnection, SqlitePool, Transaction};
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
/// 底层是 sqlx 的 `Transaction`（经 `begin_with` 指定 `BEGIN IMMEDIATE`）而不是
/// 裸执行 BEGIN：sqlx 的事务对象在 drop 时会把连接标记为需要回滚再还池，
/// future 被取消或 panic 时不会留下"带着未结事务的连接被下一个使用者复用"的隐患。
pub struct RuntimeLock {
    tx: Option<Transaction<'static, Sqlite>>,
}

impl RuntimeLock {
    pub async fn acquire(pool: &SqlitePool) -> Result<Self> {
        // IMMEDIATE 而非默认的 DEFERRED：写锁必须在读之前就拿到，
        // 否则两个进程都读完再升级写锁时会撞上 SQLITE_BUSY 死锁。
        let tx = pool.begin_with("BEGIN IMMEDIATE").await?;
        Ok(Self { tx: Some(tx) })
    }

    pub fn conn(&mut self) -> &mut SqliteConnection {
        self.tx
            .as_mut()
            .expect("RuntimeLock 在 commit/rollback 之后不应再被使用")
    }

    pub async fn commit(mut self) -> Result<()> {
        if let Some(tx) = self.tx.take() {
            tx.commit().await?;
        }
        Ok(())
    }

    pub async fn rollback(mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.rollback().await;
        }
    }
}

impl Drop for RuntimeLock {
    fn drop(&mut self) {
        if self.tx.is_some() {
            // 既没 commit 也没 rollback = 取消或 panic 路径。
            // Transaction 自己的 Drop 会安排回滚，这里只记一笔便于排查。
            warn!("RuntimeLock 未显式结束事务即被丢弃（任务取消或 panic），事务将回滚");
        }
    }
}

/// 在阻塞线程池上跑一段会 fork 子进程的同步逻辑。
///
/// `systemctl` / `sing-box check` / `openssl` 这类调用都必须走这里：
/// tokio 只配了 2 个 worker，在 async 上下文里同步等子进程会把 UI 渲染和
/// 后台流量同步一起卡住；这些调用又常常发生在持有跨进程写锁期间，
/// 阻塞时间直接转化成别的进程拿锁失败。
async fn blocking<T, F>(what: &'static str, f: F) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| anyhow!("{}任务异常终止: {}", what, e))?
}

pub async fn mutate_config_locked<T, F>(
    pool: &SqlitePool,
    config_path: &str,
    binary_path: Option<&str>,
    create_if_missing: bool,
    mutate: F,
) -> Result<T>
where
    F: FnOnce(&mut Value, &mut Vec<MetaOp>) -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    let lock = RuntimeLock::acquire(pool).await?;
    let config_path_owned = config_path.to_string();
    let binary_path_owned = binary_path.map(str::to_string);

    // 整段"读 config → 改 → 写 tmp → sing-box check → rename"里既有磁盘 IO
    // 又有子进程调用，全部挪到阻塞线程池，别占着 async worker。
    let joined = tokio::task::spawn_blocking(move || {
        let mut meta_ops: Vec<MetaOp> = Vec::new();
        let out = (|| -> Result<T> {
            let mut config_json = if Path::new(&config_path_owned).exists() {
                config::load(&config_path_owned)?
            } else if create_if_missing {
                json!({ "inbounds": [], "outbounds": [] })
            } else {
                return Err(anyhow!("config.json 不存在"));
            };
            let out = mutate(&mut config_json, &mut meta_ops)?;
            save_with_optional_validate(
                &config_path_owned,
                binary_path_owned.as_deref(),
                &config_json,
            )?;
            Ok(out)
        })();
        (out, meta_ops)
    })
    .await;

    let (result, meta_ops) = match joined {
        Ok(v) => v,
        Err(e) => {
            lock.rollback().await;
            return Err(anyhow!("配置写入任务异常终止: {}", e));
        }
    };

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
    let bin = cfg.singbox.binary_path.clone();
    let conf = cfg.singbox.config_path.clone();

    {
        let (bin, conf) = (bin.clone(), conf.clone());
        blocking("sing-box 配置校验", move || {
            SingboxProcess::new(&bin, &conf).check_config()
        })
        .await?;
    }

    let running = {
        let (bin, conf) = (bin.clone(), conf.clone());
        tokio::task::spawn_blocking(move || SingboxProcess::new(&bin, &conf).is_running())
            .await
            .unwrap_or(None)
    };

    if matches!(running, Some(true)) {
        if let Err(e) = flush_current_traffic(pool, &cfg.singbox.grpc_addr).await {
            warn!("reload 前预同步流量失败: {}", e);
        }
        blocking("sing-box reload", move || {
            SingboxProcess::new(&bin, &conf).reload()
        })
        .await?;
    }
    Ok(())
}

pub async fn apply_user_runtime_changes(pool: &SqlitePool, cfg: &AppConfig) -> Result<()> {
    if !Path::new(&cfg.singbox.config_path).exists() {
        return Ok(());
    }
    let mut lock = RuntimeLock::acquire(pool).await?;
    let users = match list_all_users(lock.conn()).await {
        Ok(users) => users,
        Err(e) => {
            lock.rollback().await;
            return Err(e);
        }
    };

    let config_path = cfg.singbox.config_path.clone();
    let binary_path = cfg.singbox.binary_path.clone();
    let grpc_addr = cfg.singbox.grpc_addr.clone();
    // sync_users 不动 meta；走同样的"save .tmp + validate + rename"流程，
    // 防止坏配置（理论上不会发生，但保险起见）覆盖主路径。
    // 里面的 sing-box check 是子进程调用，放阻塞线程池执行。
    let written = tokio::task::spawn_blocking(move || -> Result<()> {
        let mut config_json = config::load(&config_path)?;
        config::sync_users(&mut config_json, &users, &grpc_addr);
        save_with_optional_validate(&config_path, Some(&binary_path), &config_json)
    })
    .await;

    let result = match written {
        Ok(inner) => inner,
        Err(e) => Err(anyhow!("同步用户配置任务异常终止: {}", e)),
    };
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
    conn: &mut SqliteConnection,
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
            .execute(&mut *conn)
            .await?;
            sqlx::query("INSERT INTO traffic_history(username,up_bytes,down_bytes,recorded_at)VALUES(?,?,?,datetime('now'))")
                .bind(&d.username)
                .bind(d.delta_up)
                .bind(d.delta_down)
                .execute(&mut *conn)
                .await?;
        }
    }
    Ok((users, deltas))
}

async fn list_all_users(conn: &mut SqliteConnection) -> Result<Vec<User>> {
    Ok(
        sqlx::query_as::<_, User>("SELECT * FROM users ORDER BY name")
            .fetch_all(&mut *conn)
            .await?,
    )
}
