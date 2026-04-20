use anyhow::Result;
use chrono::Local;
use sqlx::SqlitePool;
use crate::model::user::User;

pub async fn list_all(pool: &SqlitePool) -> Result<Vec<User>> {
    Ok(sqlx::query_as::<_, User>("SELECT * FROM users ORDER BY name").fetch_all(pool).await?)
}

pub async fn get(pool: &SqlitePool, name: &str) -> Result<Option<User>> {
    Ok(sqlx::query_as::<_, User>("SELECT * FROM users WHERE name=?")
        .bind(name).fetch_optional(pool).await?)
}

pub async fn insert(pool: &SqlitePool, u: &User) -> Result<()> {
    // manual_bytes 在 schema 里保留为冗余列（默认 0），不再由应用写入
    sqlx::query(r#"INSERT INTO users(name,uuid,password,enabled,quota_gb,used_up_bytes,used_down_bytes,
        last_live_up,last_live_down,reset_day,last_reset_ym,
        expire_at,allow_all_nodes,created_at,allowed_nodes,sub_token)
        VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)"#)
        .bind(&u.name).bind(&u.uuid).bind(&u.password).bind(u.enabled).bind(u.quota_gb)
        .bind(u.used_up_bytes).bind(u.used_down_bytes)
        .bind(u.last_live_up).bind(u.last_live_down).bind(u.reset_day)
        .bind(&u.last_reset_ym).bind(&u.expire_at).bind(u.allow_all_nodes).bind(&u.created_at)
        .bind(&u.allowed_nodes).bind(&u.sub_token)
        .execute(pool).await?;
    Ok(())
}

pub async fn set_sub_token(pool: &SqlitePool, name: &str, token: &str) -> Result<()> {
    sqlx::query("UPDATE users SET sub_token=? WHERE name=?")
        .bind(token).bind(name)
        .execute(pool).await?;
    Ok(())
}

pub async fn find_by_token(pool: &SqlitePool, token: &str) -> Result<Option<User>> {
    Ok(sqlx::query_as::<_, User>("SELECT * FROM users WHERE sub_token=? AND sub_token != ''")
        .bind(token).fetch_optional(pool).await?)
}

pub async fn set_allow_all_nodes(pool: &SqlitePool, name: &str, allow_all: bool, allowed: &[String]) -> Result<()> {
    let json = serde_json::to_string(allowed).unwrap_or_else(|_| "[]".into());
    sqlx::query("UPDATE users SET allow_all_nodes=?, allowed_nodes=? WHERE name=?")
        .bind(allow_all).bind(&json).bind(name)
        .execute(pool).await?;
    Ok(())
}

pub async fn set_enabled(pool: &SqlitePool, name: &str, v: bool) -> Result<()> {
    sqlx::query("UPDATE users SET enabled=? WHERE name=?").bind(v).bind(name).execute(pool).await?;
    Ok(())
}

/// 原子翻转启用状态，返回新值；用户不存在返回 Ok(None)。
pub async fn toggle_enabled(pool: &SqlitePool, name: &str) -> Result<Option<bool>> {
    let mut tx = pool.begin().await?;
    let row: Option<(bool,)> = sqlx::query_as("SELECT enabled FROM users WHERE name=?")
        .bind(name).fetch_optional(&mut *tx).await?;
    let Some((cur,)) = row else { return Ok(None); };
    let next = !cur;
    sqlx::query("UPDATE users SET enabled=? WHERE name=?")
        .bind(next).bind(name).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Some(next))
}

pub async fn reset_usage(pool: &SqlitePool, name: &str) -> Result<()> {
    let ym = Local::now().format("%Y-%m").to_string();
    sqlx::query(r#"UPDATE users SET used_up_bytes=0,used_down_bytes=0,manual_bytes=0,
        last_live_up=0,last_live_down=0,last_reset_ym=? WHERE name=?"#)
        .bind(&ym).bind(name).execute(pool).await?;
    Ok(())
}

/// 合并更新套餐：None 字段保留原值。
pub async fn update_package(
    pool: &SqlitePool, name: &str,
    quota_gb: Option<f64>, reset_day: Option<i64>, expire_at: Option<&str>,
) -> Result<()> {
    sqlx::query(r#"UPDATE users SET
        quota_gb  = COALESCE(?, quota_gb),
        reset_day = COALESCE(?, reset_day),
        expire_at = COALESCE(?, expire_at)
        WHERE name = ?"#)
        .bind(quota_gb).bind(reset_day).bind(expire_at).bind(name)
        .execute(pool).await?;
    Ok(())
}
