use crate::model::user::User;
use anyhow::Result;
use chrono::Local;
use sqlx::SqlitePool;

pub async fn list_all(pool: &SqlitePool) -> Result<Vec<User>> {
    Ok(
        sqlx::query_as::<_, User>("SELECT * FROM users ORDER BY name")
            .fetch_all(pool)
            .await?,
    )
}

pub async fn get(pool: &SqlitePool, name: &str) -> Result<Option<User>> {
    Ok(
        sqlx::query_as::<_, User>("SELECT * FROM users WHERE name=?")
            .bind(name)
            .fetch_optional(pool)
            .await?,
    )
}

pub async fn insert(pool: &SqlitePool, u: &User) -> Result<()> {
    // manual_bytes 在 schema 里保留为冗余列（默认 0），不再由应用写入
    sqlx::query(
        r#"INSERT INTO users(name,uuid,password,enabled,quota_gb,used_up_bytes,used_down_bytes,
        last_live_up,last_live_down,reset_day,last_reset_ym,
        expire_at,allow_all_nodes,created_at,allowed_nodes,sub_token,traffic_multiplier,
        tg_chat_id,tg_bind_token,tg_notify_quota_80,tg_notify_quota_90,tg_notify_quota_100,
        tg_schedule_enabled,tg_schedule_times,tg_last_quota_level,tg_last_schedule_dates)
        VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)"#,
    )
    .bind(&u.name)
    .bind(&u.uuid)
    .bind(&u.password)
    .bind(u.enabled)
    .bind(u.quota_gb)
    .bind(u.used_up_bytes)
    .bind(u.used_down_bytes)
    .bind(u.last_live_up)
    .bind(u.last_live_down)
    .bind(u.reset_day)
    .bind(&u.last_reset_ym)
    .bind(&u.expire_at)
    .bind(u.allow_all_nodes)
    .bind(&u.created_at)
    .bind(&u.allowed_nodes)
    .bind(&u.sub_token)
    .bind(u.traffic_multiplier)
    .bind(u.tg_chat_id)
    .bind(&u.tg_bind_token)
    .bind(u.tg_notify_quota_80)
    .bind(u.tg_notify_quota_90)
    .bind(u.tg_notify_quota_100)
    .bind(u.tg_schedule_enabled)
    .bind(&u.tg_schedule_times)
    .bind(u.tg_last_quota_level)
    .bind(&u.tg_last_schedule_dates)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn set_sub_token(pool: &SqlitePool, name: &str, token: &str) -> Result<()> {
    sqlx::query("UPDATE users SET sub_token=? WHERE name=?")
        .bind(token)
        .bind(name)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn find_by_token(pool: &SqlitePool, token: &str) -> Result<Option<User>> {
    Ok(
        sqlx::query_as::<_, User>("SELECT * FROM users WHERE sub_token=? AND sub_token != ''")
            .bind(token)
            .fetch_optional(pool)
            .await?,
    )
}

pub async fn set_tg_bind_token(pool: &SqlitePool, name: &str, token: &str) -> Result<()> {
    sqlx::query("UPDATE users SET tg_bind_token=? WHERE name=?")
        .bind(token)
        .bind(name)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn find_by_tg_bind_token(pool: &SqlitePool, token: &str) -> Result<Option<User>> {
    Ok(sqlx::query_as::<_, User>(
        "SELECT * FROM users WHERE tg_bind_token=? AND tg_bind_token != ''",
    )
    .bind(token)
    .fetch_optional(pool)
    .await?)
}

pub async fn find_by_tg_chat_id(pool: &SqlitePool, chat_id: i64) -> Result<Option<User>> {
    Ok(
        sqlx::query_as::<_, User>("SELECT * FROM users WHERE tg_chat_id=? AND tg_chat_id != 0")
            .bind(chat_id)
            .fetch_optional(pool)
            .await?,
    )
}

pub async fn set_tg_binding(pool: &SqlitePool, name: &str, chat_id: i64) -> Result<()> {
    sqlx::query("UPDATE users SET tg_chat_id=? WHERE name=?")
        .bind(chat_id)
        .bind(name)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn clear_tg_binding_for_chat(pool: &SqlitePool, chat_id: i64) -> Result<()> {
    sqlx::query("UPDATE users SET tg_chat_id=0 WHERE tg_chat_id=?")
        .bind(chat_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_tg_notify_settings(
    pool: &SqlitePool,
    name: &str,
    notify_80: bool,
    notify_90: bool,
    notify_100: bool,
    schedule_enabled: bool,
    schedule_times_json: &str,
) -> Result<()> {
    sqlx::query(
        r#"UPDATE users SET
        tg_notify_quota_80=?,
        tg_notify_quota_90=?,
        tg_notify_quota_100=?,
        tg_schedule_enabled=?,
        tg_schedule_times=?
        WHERE name=?"#,
    )
    .bind(notify_80)
    .bind(notify_90)
    .bind(notify_100)
    .bind(schedule_enabled)
    .bind(schedule_times_json)
    .bind(name)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn set_tg_last_quota_level(pool: &SqlitePool, name: &str, level: i64) -> Result<()> {
    sqlx::query("UPDATE users SET tg_last_quota_level=? WHERE name=?")
        .bind(level)
        .bind(name)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_tg_last_schedule_dates(
    pool: &SqlitePool,
    name: &str,
    dates_json: &str,
) -> Result<()> {
    sqlx::query("UPDATE users SET tg_last_schedule_dates=? WHERE name=?")
        .bind(dates_json)
        .bind(name)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_allow_all_nodes(
    pool: &SqlitePool,
    name: &str,
    allow_all: bool,
    allowed: &[String],
) -> Result<()> {
    let json = serde_json::to_string(allowed).unwrap_or_else(|_| "[]".into());
    sqlx::query("UPDATE users SET allow_all_nodes=?, allowed_nodes=? WHERE name=?")
        .bind(allow_all)
        .bind(&json)
        .bind(name)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_enabled(pool: &SqlitePool, name: &str, v: bool) -> Result<()> {
    sqlx::query("UPDATE users SET enabled=? WHERE name=?")
        .bind(v)
        .bind(name)
        .execute(pool)
        .await?;
    Ok(())
}

/// 原子翻转启用状态，返回新值；用户不存在返回 Ok(None)。
pub async fn toggle_enabled(pool: &SqlitePool, name: &str) -> Result<Option<bool>> {
    let mut tx = pool.begin().await?;
    let row: Option<(bool,)> = sqlx::query_as("SELECT enabled FROM users WHERE name=?")
        .bind(name)
        .fetch_optional(&mut *tx)
        .await?;
    let Some((cur,)) = row else {
        return Ok(None);
    };
    let next = !cur;
    sqlx::query("UPDATE users SET enabled=? WHERE name=?")
        .bind(next)
        .bind(name)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Some(next))
}

pub async fn reset_usage(pool: &SqlitePool, name: &str) -> Result<()> {
    let ym = Local::now().format("%Y-%m").to_string();
    // 注意：不重置 last_live_up/down。
    // gRPC 流量计数器是自 sing-box 启动以来的累计值，重置后若清零 last_live，
    // 下次同步 calc_delta(gRPC累计值, 0) 会把历史累计量全部计入 used_bytes（虚报峰值）。
    // 保留 last_live 可确保增量计算 delta = 新累计 - 旧累计，只统计重置后的新增量。
    sqlx::query(
        r#"UPDATE users SET used_up_bytes=0,used_down_bytes=0,manual_bytes=0,
        last_reset_ym=? WHERE name=?"#,
    )
    .bind(&ym)
    .bind(name)
    .execute(pool)
    .await?;
    Ok(())
}

/// 手动重置：只清零流量，**不**更新 last_reset_ym。
/// 区别于 reset_usage（自动月重置用），手动重置不会污染月度去重标记，
/// 保证同月内手动重置后当月定期重置仍会在重置日正常触发。
/// 同样不重置 last_live_up/down，原因同 reset_usage。
pub async fn reset_usage_manual(pool: &SqlitePool, name: &str) -> Result<()> {
    sqlx::query(
        r#"UPDATE users SET used_up_bytes=0,used_down_bytes=0,manual_bytes=0
        WHERE name=?"#,
    )
    .bind(name)
    .execute(pool)
    .await?;
    Ok(())
}

/// 合并更新套餐：None 字段保留原值。
pub async fn update_package(
    pool: &SqlitePool,
    name: &str,
    quota_gb: Option<f64>,
    reset_day: Option<i64>,
    expire_at: Option<&str>,
    traffic_multiplier: Option<f64>,
) -> Result<()> {
    sqlx::query(
        r#"UPDATE users SET
        quota_gb  = COALESCE(?, quota_gb),
        reset_day = COALESCE(?, reset_day),
        expire_at = COALESCE(?, expire_at),
        traffic_multiplier = COALESCE(?, traffic_multiplier)
        WHERE name = ?"#,
    )
    .bind(quota_gb)
    .bind(reset_day)
    .bind(expire_at)
    .bind(traffic_multiplier)
    .bind(name)
    .execute(pool)
    .await?;
    Ok(())
}
