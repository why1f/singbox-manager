use crate::db::user_repo;
use crate::model::user::User;
use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{Datelike, Local};
use sqlx::SqlitePool;
use uuid::Uuid;

pub async fn list_users(pool: &SqlitePool) -> Result<Vec<User>> {
    user_repo::list_all(pool).await
}

/// 生成一个 32 字节随机 URL-safe token（43 字符）
pub fn new_sub_token() -> String {
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    let mut buf = [0u8; 32];
    buf[..16].copy_from_slice(a.as_bytes());
    buf[16..].copy_from_slice(b.as_bytes());
    URL_SAFE_NO_PAD.encode(buf)
}

/// 生成较短的 TG 绑定码（16 字节，22 字符）
pub fn new_tg_bind_token() -> String {
    URL_SAFE_NO_PAD.encode(Uuid::new_v4().as_bytes())
}

pub async fn add_user(
    pool: &SqlitePool,
    name: &str,
    quota_gb: f64,
    reset_day: i64,
    expire_at: &str,
    traffic_multiplier: f64,
) -> Result<User> {
    validate_username(name)?;
    validate_quota(quota_gb)?;
    validate_reset_day(reset_day)?;
    validate_expire(expire_at)?;
    validate_multiplier(traffic_multiplier)?;
    if user_repo::get(pool, name).await?.is_some() {
        return Err(anyhow!("用户 '{}' 已存在", name));
    }
    let user = User {
        name: name.into(),
        uuid: Uuid::new_v4().to_string(),
        password: Uuid::new_v4().simple().to_string(),
        enabled: true,
        quota_gb,
        used_up_bytes: 0,
        used_down_bytes: 0,
        last_live_up: 0,
        last_live_down: 0,
        reset_day,
        last_reset_ym: String::new(),
        expire_at: expire_at.into(),
        allow_all_nodes: true,
        created_at: Local::now().format("%Y-%m-%d").to_string(),
        allowed_nodes: "[]".into(),
        sub_token: new_sub_token(),
        traffic_multiplier,
        tg_chat_id: 0,
        tg_bind_token: new_tg_bind_token(),
        tg_notify_quota_80: true,
        tg_notify_quota_90: true,
        tg_notify_quota_100: true,
        tg_schedule_enabled: true,
        tg_schedule_times: "[]".into(),
        tg_last_quota_level: 0,
        tg_last_schedule_dates: "{}".into(),
        auto_disabled: false,
    };
    user_repo::insert(pool, &user).await?;
    Ok(user)
}

pub async fn regen_sub_token(pool: &SqlitePool, name: &str) -> Result<String> {
    if user_repo::get(pool, name).await?.is_none() {
        return Err(anyhow!("用户不存在: {}", name));
    }
    let t = new_sub_token();
    user_repo::set_sub_token(pool, name, &t).await?;
    Ok(t)
}

/// 撤销 token：直接置空，find_by_token 会过滤空串，/sub/ 返回 404
pub async fn revoke_sub_token(pool: &SqlitePool, name: &str) -> Result<()> {
    if user_repo::get(pool, name).await?.is_none() {
        return Err(anyhow!("用户不存在: {}", name));
    }
    user_repo::set_sub_token(pool, name, "").await?;
    Ok(())
}

pub async fn ensure_sub_tokens(pool: &SqlitePool) -> Result<usize> {
    let users = user_repo::list_all(pool).await?;
    let mut count = 0;
    for u in &users {
        if u.sub_token.is_empty() {
            let t = new_sub_token();
            user_repo::set_sub_token(pool, &u.name, &t).await?;
            count += 1;
        }
    }
    Ok(count)
}

pub async fn ensure_tg_bind_tokens(pool: &SqlitePool) -> Result<usize> {
    let users = user_repo::list_all(pool).await?;
    let mut count = 0;
    for u in &users {
        if u.tg_bind_token.is_empty() {
            let t = new_tg_bind_token();
            user_repo::set_tg_bind_token(pool, &u.name, &t).await?;
            count += 1;
        }
    }
    Ok(count)
}

pub async fn delete_user(pool: &SqlitePool, name: &str) -> Result<()> {
    if name == "admin" {
        return Err(anyhow!("不能删除 admin"));
    }
    if user_repo::get(pool, name).await?.is_none() {
        return Err(anyhow!("用户不存在: {}", name));
    }
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM users WHERE name=?")
        .bind(name)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM traffic_history WHERE username=?")
        .bind(name)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn toggle_user(pool: &SqlitePool, name: &str) -> Result<bool> {
    user_repo::toggle_enabled(pool, name)
        .await?
        .ok_or_else(|| anyhow!("用户不存在: {}", name))
}

pub async fn reset_traffic(pool: &SqlitePool, name: &str) -> Result<()> {
    // 手动重置：不写 last_reset_ym，避免污染本月定期重置的去重标记
    let user = user_repo::get(pool, name)
        .await?
        .ok_or_else(|| anyhow!("用户不存在: {}", name))?;
    user_repo::reset_usage_manual(pool, name).await?;
    // 因超额被系统自动禁用的用户，清零后应立即恢复可用——否则要等到下个重置日。
    // 管理员手动停用的（auto_disabled=false）不动。
    if !user.enabled && user.auto_disabled {
        user_repo::set_enabled_auto(pool, name, true).await?;
    }
    Ok(())
}

/// TG 绑定码：重新生成（旧码立即失效，已绑定的 chat 不受影响）。
pub async fn regen_tg_bind_token(pool: &SqlitePool, name: &str) -> Result<String> {
    if user_repo::get(pool, name).await?.is_none() {
        return Err(anyhow!("用户不存在: {}", name));
    }
    let token = new_tg_bind_token();
    user_repo::set_tg_bind_token(pool, name, &token).await?;
    Ok(token)
}

/// 解除该用户的 TG 绑定，返回被解绑的 chat_id（原本未绑定时返回 None）。
pub async fn unbind_tg(pool: &SqlitePool, name: &str) -> Result<Option<i64>> {
    let user = user_repo::get(pool, name)
        .await?
        .ok_or_else(|| anyhow!("用户不存在: {}", name))?;
    if !user.tg_is_bound() {
        return Ok(None);
    }
    user_repo::set_tg_binding(pool, name, 0).await?;
    Ok(Some(user.tg_chat_id))
}

/// 绑定成功后作废绑定码：绑定码是一次性的，避免泄露后被重放抢绑。
pub async fn consume_tg_bind_token(pool: &SqlitePool, name: &str) -> Result<()> {
    user_repo::set_tg_bind_token(pool, name, "").await
}

pub async fn update_package(
    pool: &SqlitePool,
    name: &str,
    quota_gb: Option<f64>,
    reset_day: Option<i64>,
    expire_at: Option<&str>,
    traffic_multiplier: Option<f64>,
) -> Result<()> {
    if user_repo::get(pool, name).await?.is_none() {
        return Err(anyhow!("用户不存在: {}", name));
    }
    if quota_gb.is_none()
        && reset_day.is_none()
        && expire_at.is_none()
        && traffic_multiplier.is_none()
    {
        return Ok(());
    }
    if let Some(v) = quota_gb {
        validate_quota(v)?;
    }
    if let Some(v) = reset_day {
        validate_reset_day(v)?;
    }
    if let Some(v) = expire_at {
        validate_expire(v)?;
    }
    if let Some(v) = traffic_multiplier {
        validate_multiplier(v)?;
    }
    user_repo::update_package(
        pool,
        name,
        quota_gb,
        reset_day,
        expire_at,
        traffic_multiplier,
    )
    .await
}

/// 允许用户访问指定节点 tag。若当前是全开状态，自动切换为按列表授权。
pub async fn grant_node(pool: &SqlitePool, name: &str, tag: &str) -> Result<()> {
    let user = user_repo::get(pool, name)
        .await?
        .ok_or_else(|| anyhow!("用户不存在: {}", name))?;
    let mut list = user.allowed_tags();
    if !list.iter().any(|t| t == tag) {
        list.push(tag.to_string());
    }
    user_repo::set_allow_all_nodes(pool, name, false, &list).await
}

/// 取消用户对指定节点 tag 的访问。若当前全开，按需计算"除此之外全部"语义。
pub async fn revoke_node(
    pool: &SqlitePool,
    name: &str,
    tag: &str,
    all_existing_tags: &[String],
) -> Result<()> {
    let user = user_repo::get(pool, name)
        .await?
        .ok_or_else(|| anyhow!("用户不存在: {}", name))?;
    let list: Vec<String> = if user.allow_all_nodes {
        all_existing_tags
            .iter()
            .filter(|t| *t != tag)
            .cloned()
            .collect()
    } else {
        user.allowed_tags()
            .into_iter()
            .filter(|t| t != tag)
            .collect()
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
pub async fn set_allowed_tags(pool: &SqlitePool, name: &str, tags: &[String]) -> Result<()> {
    if user_repo::get(pool, name).await?.is_none() {
        return Err(anyhow!("用户不存在: {}", name));
    }
    user_repo::set_allow_all_nodes(pool, name, false, tags).await
}

/// 节点删除后，清理所有用户 allowed_nodes 里的残留 tag，避免界面出现脏引用。
pub async fn remove_allowed_tag_from_all_users(pool: &SqlitePool, tag: &str) -> Result<usize> {
    let users = user_repo::list_all(pool).await?;
    let mut cleaned = 0usize;
    for user in users.into_iter().filter(|u| !u.allow_all_nodes) {
        let mut tags = user.allowed_tags();
        let before = tags.len();
        tags.retain(|item| item != tag);
        if tags.len() != before {
            user_repo::set_allow_all_nodes(pool, &user.name, false, &tags).await?;
            cleaned += 1;
        }
    }
    Ok(cleaned)
}

pub async fn apply_automatic_controls(pool: &SqlitePool) -> Result<Vec<String>> {
    let users = user_repo::list_all(pool).await?;
    let today = Local::now().date_naive();
    let ym = today.format("%Y-%m").to_string();
    let day = today.day() as i64;
    let last_d = last_day_of_month(today);
    let mut changed = Vec::new();
    for user in &users {
        if user.is_expired() && user.enabled {
            user_repo::set_enabled_auto(pool, &user.name, false).await?;
            changed.push(format!("{}(到期禁用)", user.name));
            continue;
        }
        let eff = match user.reset_day {
            32 => last_d,
            d @ 1..=31 => d.min(last_d),
            _ => 0,
        };
        if eff > 0 && day == eff && user.last_reset_ym != ym {
            user_repo::reset_usage(pool, &user.name).await?;
            // 只解封"系统自动禁用"的用户（超额），管理员手动停用的保持停用。
            // 到期禁用的用户已在上面 continue 跳过，不会走到这里。
            if !user.enabled && user.auto_disabled {
                user_repo::set_enabled_auto(pool, &user.name, true).await?;
                changed.push(format!("{}(月重置+解封)", user.name));
            } else {
                changed.push(format!("{}(月重置)", user.name));
            }
            continue; // 跳过本轮超额检查，流量刚清零不应立刻再被禁
        }
        if user.is_over_quota() && user.enabled {
            user_repo::set_enabled_auto(pool, &user.name, false).await?;
            changed.push(format!("{}(超额禁用)", user.name));
        }
    }
    Ok(changed)
}

fn validate_username(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(anyhow!("用户名不能为空"));
    }
    if name == "admin" {
        return Err(anyhow!("'admin' 为保留名"));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(anyhow!("用户名只能含字母/数字/-/_"));
    }
    if name.len() > 32 {
        return Err(anyhow!("用户名不超过 32 字符"));
    }
    Ok(())
}

fn validate_quota(quota_gb: f64) -> Result<()> {
    if quota_gb < 0.0 {
        return Err(anyhow!("配额不能为负数"));
    }
    Ok(())
}

fn validate_reset_day(reset_day: i64) -> Result<()> {
    if reset_day == 0 || reset_day == 32 || (1..=31).contains(&reset_day) {
        return Ok(());
    }
    Err(anyhow!("重置日需为 0 / 1-31 / 32(月末)"))
}

fn validate_expire(expire_at: &str) -> Result<()> {
    if expire_at.is_empty() {
        return Ok(());
    }
    chrono::NaiveDate::parse_from_str(expire_at, "%Y-%m-%d")
        .map(|_| ())
        .map_err(|_| anyhow!("到期日格式需为 YYYY-MM-DD"))
}

fn validate_multiplier(multiplier: f64) -> Result<()> {
    if multiplier < 0.0 {
        return Err(anyhow!("倍率不能为负数"));
    }
    Ok(())
}

fn last_day_of_month(d: chrono::NaiveDate) -> i64 {
    let next = if d.month() == 12 {
        chrono::NaiveDate::from_ymd_opt(d.year() + 1, 1, 1)
    } else {
        chrono::NaiveDate::from_ymd_opt(d.year(), d.month() + 1, 1)
    };
    next.and_then(|n| n.pred_opt())
        .map(|d| d.day() as i64)
        .unwrap_or(30)
}

#[cfg(test)]
mod tests {
    use super::{
        add_user, apply_automatic_controls, consume_tg_bind_token, delete_user, last_day_of_month,
        regen_tg_bind_token, reset_traffic, unbind_tg, update_package, validate_expire,
        validate_multiplier, validate_quota, validate_reset_day,
    };
    use crate::db::user_repo;
    use chrono::NaiveDate;
    use std::path::PathBuf;
    use uuid::Uuid;

    async fn temp_pool() -> sqlx::SqlitePool {
        let path: PathBuf = std::env::temp_dir().join(format!("sbm-test-{}.db", Uuid::new_v4()));
        crate::db::init_pool(path.to_string_lossy().as_ref())
            .await
            .unwrap()
    }

    #[test]
    fn reset_day_accepts_31() {
        assert!(validate_reset_day(31).is_ok());
        assert!(validate_reset_day(32).is_ok());
        assert!(validate_reset_day(33).is_err());
    }

    #[test]
    fn expire_requires_valid_date() {
        assert!(validate_expire("").is_ok());
        assert!(validate_expire("2026-12-31").is_ok());
        assert!(validate_expire("2026-02-30").is_err());
    }

    #[test]
    fn numeric_fields_reject_negative_values() {
        assert!(validate_quota(-1.0).is_err());
        assert!(validate_multiplier(-0.1).is_err());
    }

    #[test]
    fn last_day_of_month_handles_february() {
        let date = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();
        assert_eq!(last_day_of_month(date), 28);
    }

    #[tokio::test]
    async fn delete_missing_user_returns_error() {
        let pool = temp_pool().await;
        let err = delete_user(&pool, "missing").await.unwrap_err();
        assert!(err.to_string().contains("用户不存在"));
    }

    #[tokio::test]
    async fn reset_missing_user_returns_error() {
        let pool = temp_pool().await;
        let err = reset_traffic(&pool, "missing").await.unwrap_err();
        assert!(err.to_string().contains("用户不存在"));
    }

    #[tokio::test]
    async fn update_missing_user_returns_error() {
        let pool = temp_pool().await;
        let err = update_package(&pool, "missing", Some(1.0), None, None, None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("用户不存在"));
    }

    /// 管理员手动停用的用户，在重置日不能被自动解封。
    #[tokio::test]
    async fn monthly_reset_does_not_revive_manually_disabled_user() {
        let pool = temp_pool().await;
        let today = chrono::Local::now().date_naive();
        let reset_day = chrono::Datelike::day(&today) as i64;

        add_user(&pool, "manual", 10.0, reset_day, "", 1.0)
            .await
            .unwrap();
        add_user(&pool, "overquota", 10.0, reset_day, "", 1.0)
            .await
            .unwrap();

        // manual：管理员手动停用（set_enabled 会清 auto_disabled）
        user_repo::set_enabled(&pool, "manual", false)
            .await
            .unwrap();
        // overquota：系统因超额自动停用
        user_repo::set_enabled_auto(&pool, "overquota", false)
            .await
            .unwrap();

        apply_automatic_controls(&pool).await.unwrap();

        let manual = user_repo::get(&pool, "manual").await.unwrap().unwrap();
        let auto = user_repo::get(&pool, "overquota").await.unwrap().unwrap();
        assert!(!manual.enabled, "手动停用的用户不应被月重置解封");
        assert!(auto.enabled, "超额自动停用的用户应在月重置日解封");
    }

    /// 手动清零流量后，超额被自动禁用的用户应立刻恢复；手动停用的保持停用。
    #[tokio::test]
    async fn manual_reset_revives_only_auto_disabled_user() {
        let pool = temp_pool().await;
        add_user(&pool, "auto", 1.0, 0, "", 1.0).await.unwrap();
        add_user(&pool, "manual", 1.0, 0, "", 1.0).await.unwrap();
        user_repo::set_enabled_auto(&pool, "auto", false)
            .await
            .unwrap();
        user_repo::set_enabled(&pool, "manual", false)
            .await
            .unwrap();

        reset_traffic(&pool, "auto").await.unwrap();
        reset_traffic(&pool, "manual").await.unwrap();

        assert!(
            user_repo::get(&pool, "auto")
                .await
                .unwrap()
                .unwrap()
                .enabled
        );
        assert!(
            !user_repo::get(&pool, "manual")
                .await
                .unwrap()
                .unwrap()
                .enabled
        );
    }

    /// 绑定码一次性：核销后查不到，重生成后是新码。
    #[tokio::test]
    async fn bind_token_is_single_use_and_regenerable() {
        let pool = temp_pool().await;
        let user = add_user(&pool, "alice", 0.0, 0, "", 1.0).await.unwrap();
        let original = user.tg_bind_token.clone();
        assert!(!original.is_empty());
        assert!(user_repo::find_by_tg_bind_token(&pool, &original)
            .await
            .unwrap()
            .is_some());

        consume_tg_bind_token(&pool, "alice").await.unwrap();
        assert!(
            user_repo::find_by_tg_bind_token(&pool, &original)
                .await
                .unwrap()
                .is_none(),
            "核销后旧码不应还能用来绑定"
        );

        let fresh = regen_tg_bind_token(&pool, "alice").await.unwrap();
        assert_ne!(fresh, original);
        assert!(user_repo::find_by_tg_bind_token(&pool, &fresh)
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn unbind_reports_previous_chat_and_is_idempotent() {
        let pool = temp_pool().await;
        add_user(&pool, "alice", 0.0, 0, "", 1.0).await.unwrap();
        assert_eq!(unbind_tg(&pool, "alice").await.unwrap(), None);

        user_repo::set_tg_binding(&pool, "alice", 12345)
            .await
            .unwrap();
        assert_eq!(unbind_tg(&pool, "alice").await.unwrap(), Some(12345));
        assert_eq!(unbind_tg(&pool, "alice").await.unwrap(), None);
    }
}
