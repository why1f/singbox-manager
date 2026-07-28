use anyhow::Result;
use sqlx::SqlitePool;

use crate::model::telegram::TgAdminPrefs;

pub async fn ensure_admin_pref(
    pool: &SqlitePool,
    chat_id: i64,
    notify_quota: bool,
    schedule_enabled: bool,
    schedule_times_json: &str,
) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO tg_admin_prefs(chat_id, notify_quota, schedule_enabled, schedule_times)
        VALUES(?, ?, ?, ?)
        ON CONFLICT(chat_id) DO NOTHING"#,
    )
    .bind(chat_id)
    .bind(notify_quota)
    .bind(schedule_enabled)
    .bind(schedule_times_json)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_admin_pref(pool: &SqlitePool, chat_id: i64) -> Result<Option<TgAdminPrefs>> {
    Ok(
        sqlx::query_as::<_, TgAdminPrefs>("SELECT * FROM tg_admin_prefs WHERE chat_id=?")
            .bind(chat_id)
            .fetch_optional(pool)
            .await?,
    )
}

pub async fn list_admin_prefs(pool: &SqlitePool, chat_ids: &[i64]) -> Result<Vec<TgAdminPrefs>> {
    let mut out = Vec::new();
    for chat_id in chat_ids {
        if let Some(item) = get_admin_pref(pool, *chat_id).await? {
            out.push(item);
        }
    }
    Ok(out)
}

pub async fn set_admin_notify_quota(pool: &SqlitePool, chat_id: i64, enabled: bool) -> Result<()> {
    sqlx::query("UPDATE tg_admin_prefs SET notify_quota=? WHERE chat_id=?")
        .bind(enabled)
        .bind(chat_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_admin_schedule(
    pool: &SqlitePool,
    chat_id: i64,
    enabled: bool,
    schedule_times_json: &str,
) -> Result<()> {
    sqlx::query("UPDATE tg_admin_prefs SET schedule_enabled=?, schedule_times=? WHERE chat_id=?")
        .bind(enabled)
        .bind(schedule_times_json)
        .bind(chat_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_admin_last_schedule_dates(
    pool: &SqlitePool,
    chat_id: i64,
    dates_json: &str,
) -> Result<()> {
    sqlx::query("UPDATE tg_admin_prefs SET last_schedule_dates=? WHERE chat_id=?")
        .bind(dates_json)
        .bind(chat_id)
        .execute(pool)
        .await?;
    Ok(())
}

// ——— Telegram Bot 单实例租约 ———
//
// 同一个 bot_token 只能有一个 getUpdates 长轮询在跑，多开会互相抢 update。
// daemon 和 TUI 都可能想启动 bot，用这张单行表做跨进程互斥。

/// 尝试拿下 bot 租约。空闲、已属于自己、或持有者心跳超过 `stale_secs` 未更新时成功。
/// 整个判断在一条 UPSERT 里完成，多进程同时抢也不会都成功。
pub async fn try_acquire_bot_lease(
    pool: &SqlitePool,
    owner: &str,
    stale_secs: i64,
) -> Result<bool> {
    let cutoff = format!("-{} seconds", stale_secs.max(1));
    let res = sqlx::query(
        r#"INSERT INTO tg_bot_lease(id, owner, heartbeat) VALUES(1, ?, datetime('now'))
        ON CONFLICT(id) DO UPDATE SET owner = excluded.owner, heartbeat = excluded.heartbeat
        WHERE tg_bot_lease.owner = excluded.owner
           OR tg_bot_lease.heartbeat <= datetime('now', ?)"#,
    )
    .bind(owner)
    .bind(&cutoff)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// 续租。返回 false 表示租约已被别人接管，调用方应停掉自己的 bot。
pub async fn renew_bot_lease(pool: &SqlitePool, owner: &str) -> Result<bool> {
    let res =
        sqlx::query("UPDATE tg_bot_lease SET heartbeat=datetime('now') WHERE id=1 AND owner=?")
            .bind(owner)
            .execute(pool)
            .await?;
    Ok(res.rows_affected() > 0)
}

/// 当前租约持有者，用于把"谁占着"写进日志。
pub async fn bot_lease_holder(pool: &SqlitePool) -> Result<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as("SELECT owner FROM tg_bot_lease WHERE id=1")
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|(owner,)| owner))
}
