use anyhow::{anyhow, Result};
use chrono::{Datelike, Local};
use sqlx::SqlitePool;
use uuid::Uuid;
use crate::db::user_repo;
use crate::model::user::User;

pub async fn list_users(pool: &SqlitePool) -> Result<Vec<User>> { user_repo::list_all(pool).await }

pub async fn add_user(pool: &SqlitePool, name: &str, quota_gb: f64,
    reset_day: i64, expire_at: &str) -> Result<User> {
    validate_username(name)?;
    if user_repo::get(pool, name).await?.is_some() {
        return Err(anyhow!("用户 '{}' 已存在", name));
    }
    let user = User {
        name: name.into(),
        uuid: Uuid::new_v4().to_string(),
        password: Uuid::new_v4().simple().to_string(),
        enabled: true, quota_gb,
        used_up_bytes: 0, used_down_bytes: 0, manual_bytes: 0,
        last_live_up: 0, last_live_down: 0, reset_day,
        last_reset_ym: String::new(),
        expire_at: expire_at.into(),
        allow_all_nodes: true,
        created_at: Local::now().format("%Y-%m-%d").to_string(),
        allowed_nodes: "[]".into(),
    };
    user_repo::insert(pool, &user).await?;
    Ok(user)
}

pub async fn delete_user(pool: &SqlitePool, name: &str) -> Result<()> {
    if name == "admin" { return Err(anyhow!("不能删除 admin")); }
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM users WHERE name=?").bind(name).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM traffic_history WHERE username=?").bind(name).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn toggle_user(pool: &SqlitePool, name: &str) -> Result<bool> {
    user_repo::toggle_enabled(pool, name).await?
        .ok_or_else(|| anyhow!("用户不存在: {}", name))
}

pub async fn reset_traffic(pool: &SqlitePool, name: &str) -> Result<()> {
    user_repo::reset_usage(pool, name).await
}

pub async fn update_package(pool: &SqlitePool, name: &str,
    quota_gb: Option<f64>, reset_day: Option<i64>, expire_at: Option<&str>) -> Result<()> {
    if quota_gb.is_none() && reset_day.is_none() && expire_at.is_none() { return Ok(()); }
    user_repo::update_package(pool, name, quota_gb, reset_day, expire_at).await
}

/// 允许用户访问指定节点 tag。若当前是全开状态，自动切换为按列表授权。
pub async fn grant_node(pool: &SqlitePool, name: &str, tag: &str) -> Result<()> {
    let user = user_repo::get(pool, name).await?
        .ok_or_else(|| anyhow!("用户不存在: {}", name))?;
    let mut list = user.allowed_tags();
    if !list.iter().any(|t| t == tag) { list.push(tag.to_string()); }
    user_repo::set_allow_all_nodes(pool, name, false, &list).await
}

/// 取消用户对指定节点 tag 的访问。若当前全开，按需计算"除此之外全部"语义。
pub async fn revoke_node(pool: &SqlitePool, name: &str, tag: &str, all_existing_tags: &[String]) -> Result<()> {
    let user = user_repo::get(pool, name).await?
        .ok_or_else(|| anyhow!("用户不存在: {}", name))?;
    let list: Vec<String> = if user.allow_all_nodes {
        all_existing_tags.iter().filter(|t| *t != tag).cloned().collect()
    } else {
        user.allowed_tags().into_iter().filter(|t| t != tag).collect()
    };
    user_repo::set_allow_all_nodes(pool, name, false, &list).await
}

/// 恢复为全部节点可用
pub async fn grant_all_nodes(pool: &SqlitePool, name: &str) -> Result<()> {
    if user_repo::get(pool, name).await?.is_none() {
        return Err(anyhow!("用户不存在: {}", name));
    }
    user_repo::set_allow_all_nodes(pool, name, true, &[]).await
}

/// 直接设置允许列表（覆盖式）
#[allow(dead_code)]
pub async fn set_allowed_tags(pool: &SqlitePool, name: &str, tags: &[String]) -> Result<()> {
    if user_repo::get(pool, name).await?.is_none() {
        return Err(anyhow!("用户不存在: {}", name));
    }
    user_repo::set_allow_all_nodes(pool, name, false, tags).await
}

pub async fn apply_automatic_controls(pool: &SqlitePool) -> Result<Vec<String>> {
    let users = user_repo::list_all(pool).await?;
    let today = Local::now().date_naive();
    let ym    = today.format("%Y-%m").to_string();
    let day   = today.day() as i64;
    let last_d = last_day_of_month(today);
    let mut changed = Vec::new();
    for user in &users {
        if user.is_expired() && user.enabled {
            user_repo::set_enabled(pool, &user.name, false).await?;
            changed.push(format!("{}(到期禁用)", user.name));
            continue;
        }
        let eff = match user.reset_day { 32 => last_d, d @ 1..=28 => d.min(last_d), _ => 0 };
        if eff > 0 && day == eff && user.last_reset_ym != ym {
            user_repo::reset_usage(pool, &user.name).await?;
            changed.push(format!("{}(月重置)", user.name));
        }
        if user.is_over_quota() && user.enabled {
            user_repo::set_enabled(pool, &user.name, false).await?;
            changed.push(format!("{}(超额禁用)", user.name));
        }
    }
    Ok(changed)
}

fn validate_username(name: &str) -> Result<()> {
    if name.is_empty() { return Err(anyhow!("用户名不能为空")); }
    if name == "admin" { return Err(anyhow!("'admin' 为保留名")); }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(anyhow!("用户名只能含字母/数字/-/_"));
    }
    if name.len() > 32 { return Err(anyhow!("用户名不超过 32 字符")); }
    Ok(())
}

fn last_day_of_month(d: chrono::NaiveDate) -> i64 {
    let next = if d.month() == 12 {
        chrono::NaiveDate::from_ymd_opt(d.year() + 1, 1, 1)
    } else {
        chrono::NaiveDate::from_ymd_opt(d.year(), d.month() + 1, 1)
    };
    next.and_then(|n| n.pred_opt()).map(|d| d.day() as i64).unwrap_or(30)
}
