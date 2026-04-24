use crate::core::{
    grpc::{query_all_traffic, StatsClient},
    traffic::calc_deltas,
};
use crate::db::{traffic_repo, user_repo};
use crate::model::{config::AppConfig, traffic::TrafficDelta, user::User};
use crate::service::{runtime_service, user_service::apply_automatic_controls};
use anyhow::Result;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{error, warn};

#[derive(Debug, Clone)]
pub enum TrafficEvent {
    Synced(Vec<TrafficDelta>),
    QuotaAlert(String, u8),
    AutoControl(Vec<String>),
    GrpcError(String),
    GrpcConnected,
    RuntimeSyncError(String),
    Tick,
}

/// 跑到 gRPC 失败/断开（由调用方决定是否重连）。不会重连自身。
pub async fn run_until_disconnect(
    pool: SqlitePool,
    mut grpc: StatsClient,
    sync_secs: u64,
    alert_pct: u8,
    cfg: Arc<AppConfig>,
    tx: mpsc::Sender<TrafficEvent>,
) {
    let mut siv = tokio::time::interval(Duration::from_secs(sync_secs.max(1)));
    let mut tiv = tokio::time::interval(Duration::from_secs(1));
    let mut civ = tokio::time::interval(Duration::from_secs(60));
    let mut hiv = tokio::time::interval(Duration::from_secs(3600));
    siv.tick().await;
    civ.tick().await;
    hiv.tick().await;

    let mut alerted: HashMap<String, u8> = HashMap::new();
    let mut runtime_sync_dirty = false;

    let _ = tx.send(TrafficEvent::GrpcConnected).await;
    loop {
        tokio::select! {
            _ = tiv.tick() => { if tx.send(TrafficEvent::Tick).await.is_err() { return; } }
            _ = siv.tick() => {
                match sync_once(&pool, &mut grpc, alert_pct, &mut alerted, &tx).await {
                    Ok(()) => {}
                    Err(e) => {
                        warn!("流量同步失败: {}", e);
                        let _ = tx.send(TrafficEvent::GrpcError(e.to_string())).await;
                        return;
                    }
                }
            }
            _ = civ.tick() => {
                match apply_automatic_controls(&pool).await {
                    Ok(c) => {
                        if !c.is_empty() {
                            runtime_sync_dirty = true;
                        }
                        if runtime_sync_dirty {
                            match runtime_service::apply_user_runtime_changes(&pool, &cfg).await {
                                Ok(()) => {
                                    runtime_sync_dirty = false;
                                    if !c.is_empty() {
                                        let _ = tx.send(TrafficEvent::AutoControl(c)).await;
                                    }
                                }
                                Err(e) => {
                                    error!("自动控制配置同步失败: {}", e);
                                    let _ = tx.send(TrafficEvent::RuntimeSyncError(e.to_string())).await;
                                }
                            }
                        }
                    }
                    Err(e) => error!("自动控制: {}", e),
                }
            }
            _ = hiv.tick() => {
                if let Err(e) = traffic_repo::cleanup_old(&pool).await {
                    warn!("清理流量历史失败: {}", e);
                }
            }
        }
    }
}

pub async fn flush_current_traffic(
    pool: &SqlitePool,
    grpc_addr: &str,
) -> Result<Vec<TrafficDelta>> {
    let mut grpc = crate::core::grpc::connect(grpc_addr).await?;
    sync_current_traffic(pool, &mut grpc).await
}

async fn sync_once(
    pool: &SqlitePool,
    grpc: &mut StatsClient,
    alert_pct: u8,
    alerted: &mut HashMap<String, u8>,
    tx: &mpsc::Sender<TrafficEvent>,
) -> Result<()> {
    let (users, deltas) = sync_current_traffic_with_users(pool, grpc).await?;

    // 告警去重：阈值档位变化才发送（80 / 90 / 100）
    for u in &users {
        if u.quota_gb <= 0.0 {
            continue;
        }
        let applied_up = u.used_up_bytes
            + deltas
                .iter()
                .find(|d| d.username == u.name)
                .map(|d| d.delta_up)
                .unwrap_or(0);
        let applied_dn = u.used_down_bytes
            + deltas
                .iter()
                .find(|d| d.username == u.name)
                .map(|d| d.delta_down)
                .unwrap_or(0);
        let used = ((applied_up + applied_dn) as f64 * u.traffic_multiplier) as i64;
        let quota = (u.quota_gb * 1_073_741_824.0) as i64;
        if quota <= 0 {
            continue;
        }
        let pct = ((used as f64 / quota as f64 * 100.0).min(100.0)) as u8;
        let bucket = if pct >= 100 {
            100
        } else if pct >= 95 {
            95
        } else if pct >= alert_pct {
            alert_pct
        } else {
            0
        };
        if bucket == 0 {
            alerted.remove(&u.name);
            continue;
        }
        if alerted.get(&u.name).copied() != Some(bucket) {
            alerted.insert(u.name.clone(), bucket);
            let _ = tx.send(TrafficEvent::QuotaAlert(u.name.clone(), pct)).await;
        }
    }

    if !deltas.is_empty() {
        let _ = tx.send(TrafficEvent::Synced(deltas)).await;
    }
    Ok(())
}

async fn sync_current_traffic_with_users(
    pool: &SqlitePool,
    grpc: &mut StatsClient,
) -> Result<(Vec<User>, Vec<TrafficDelta>)> {
    let snaps = query_all_traffic(grpc, false).await?;
    let users = user_repo::list_all(pool).await?;
    let deltas = calc_deltas(&snaps, &users);
    if !deltas.is_empty() {
        let mut tx_db = pool.begin().await?;
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
            .execute(&mut *tx_db)
            .await?;
            sqlx::query("INSERT INTO traffic_history(username,up_bytes,down_bytes,recorded_at)VALUES(?,?,?,datetime('now'))")
                .bind(&d.username).bind(d.delta_up).bind(d.delta_down)
                .execute(&mut *tx_db).await?;
        }
        tx_db.commit().await?;
    }
    Ok((users, deltas))
}

async fn sync_current_traffic(
    pool: &SqlitePool,
    grpc: &mut StatsClient,
) -> Result<Vec<TrafficDelta>> {
    let (_, deltas) = sync_current_traffic_with_users(pool, grpc).await?;
    Ok(deltas)
}
