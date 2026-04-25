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
