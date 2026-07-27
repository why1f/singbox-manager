use crate::core::grpc::StatsClient;
use crate::db::traffic_repo;
use crate::model::{config::AppConfig, traffic::TrafficDelta};
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

/// 需要**跨 gRPC 重连**保持的状态。
///
/// 这两项以前都是 `run_until_disconnect` 的局部变量，每次断线重连就清空，带来两个问题：
/// - `runtime_sync_dirty` 丢失 → "DB 已禁用、config.json 仍放行"的失配无限期持续，
///   直到下一次任意用户变更才被动修复；
/// - `alerted` 丢失 → 已经告警过的用户在每次重连后被重复推送一次。
#[derive(Default)]
pub struct SyncState {
    /// 用户名 -> 已告警的档位，档位变化才推送
    alerted: HashMap<String, u8>,
    /// DB 已被自动控制改动，但 config.json 尚未同步成功
    runtime_sync_dirty: bool,
}

impl SyncState {
    pub fn new() -> Self {
        Self::default()
    }

    /// 标记 DB 有未同步到 config.json 的改动
    pub fn mark_dirty(&mut self) {
        self.runtime_sync_dirty = true;
    }

    pub fn is_dirty(&self) -> bool {
        self.runtime_sync_dirty
    }

    /// 尝试把 DB 状态同步进 config.json 并 reload；成功才清除脏标记。
    /// 无脏标记时直接返回 Ok(())，不做任何 IO。
    pub async fn sync_if_dirty(&mut self, pool: &SqlitePool, cfg: &AppConfig) -> Result<()> {
        if !self.runtime_sync_dirty {
            return Ok(());
        }
        runtime_service::apply_user_runtime_changes(pool, cfg).await?;
        self.runtime_sync_dirty = false;
        Ok(())
    }
}

/// 跑到 gRPC 失败/断开（由调用方决定是否重连）。不会重连自身。
pub async fn run_until_disconnect(
    pool: SqlitePool,
    mut grpc: StatsClient,
    sync_secs: u64,
    alert_pct: u8,
    cfg: Arc<AppConfig>,
    tx: mpsc::Sender<TrafficEvent>,
    state: &mut SyncState,
) {
    let mut siv = tokio::time::interval(Duration::from_secs(sync_secs.max(1)));
    let mut tiv = tokio::time::interval(Duration::from_secs(1));
    let mut civ = tokio::time::interval(Duration::from_secs(60));
    let mut hiv = tokio::time::interval(Duration::from_secs(3600));
    siv.tick().await;
    civ.tick().await;
    hiv.tick().await;

    let _ = tx.send(TrafficEvent::GrpcConnected).await;
    loop {
        tokio::select! {
            _ = tiv.tick() => { if tx.send(TrafficEvent::Tick).await.is_err() { return; } }
            _ = siv.tick() => {
                match sync_once(&pool, &mut grpc, alert_pct, &mut state.alerted, &tx).await {
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
                            state.mark_dirty();
                        }
                        if state.is_dirty() {
                            match state.sync_if_dirty(&pool, &cfg).await {
                                Ok(()) => {
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
    runtime_service::flush_current_traffic(pool, grpc_addr).await
}

/// 告警档位。与 TG 侧 `quota_level`（100 / 90 / 80）严格对齐——
/// 历史上这里是 100/95/alert_pct，导致用户设置的"90% 提醒"实际要到 95% 才发出。
/// `alert_pct > 90` 时不启用 90 档，尊重管理员把首档抬高的意图。
fn alert_bucket(pct: u8, alert_pct: u8) -> u8 {
    if pct >= 100 {
        100
    } else if pct >= 90 && alert_pct <= 90 {
        90
    } else if pct >= alert_pct {
        alert_pct
    } else {
        0
    }
}

async fn sync_once(
    pool: &SqlitePool,
    grpc: &mut StatsClient,
    alert_pct: u8,
    alerted: &mut HashMap<String, u8>,
    tx: &mpsc::Sender<TrafficEvent>,
) -> Result<()> {
    let (users, deltas) = runtime_service::sync_current_traffic_with_users(pool, grpc).await?;

    // 告警去重：阈值档位变化才发送
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
        let bucket = alert_bucket(pct, alert_pct);
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

#[cfg(test)]
mod tests {
    use super::alert_bucket;

    #[test]
    fn buckets_align_with_telegram_levels() {
        assert_eq!(alert_bucket(100, 80), 100);
        assert_eq!(alert_bucket(93, 80), 90, "90-99% 应落在 90 档而非 95");
        assert_eq!(alert_bucket(85, 80), 80);
        assert_eq!(alert_bucket(79, 80), 0);
    }

    #[test]
    fn custom_alert_pct_above_90_disables_the_90_bucket() {
        assert_eq!(
            alert_bucket(92, 95),
            0,
            "管理员把首档设到 95 就不该在 90 触发"
        );
        assert_eq!(alert_bucket(96, 95), 95);
        assert_eq!(alert_bucket(100, 95), 100);
    }
}
