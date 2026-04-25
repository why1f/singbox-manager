use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, FixedOffset, Timelike, Utc};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::SqlitePool;
use tokio::sync::{mpsc, Mutex};
use tracing::warn;

use crate::{
    db::{tg_repo, user_repo},
    model::{config::AppConfig, telegram::TgAdminPrefs, user::User},
};

pub enum TgEvent {
    QuotaAlert { username: String, percent: u8 },
}

#[derive(Clone)]
struct TgContext {
    pool: SqlitePool,
    cfg: Arc<AppConfig>,
    client: Client,
    offset: FixedOffset,
    pending_inputs: Arc<Mutex<HashMap<i64, PendingInput>>>,
}

#[derive(Clone)]
enum PendingInput {
    UserSchedule { username: String },
    AdminSchedule,
}

#[derive(Debug, Deserialize)]
struct TgResponse<T> {
    ok: bool,
    result: T,
}

#[derive(Debug, Deserialize)]
struct TgUpdate {
    update_id: i64,
    #[serde(default)]
    message: Option<TgMessage>,
    #[serde(default)]
    callback_query: Option<TgCallbackQuery>,
}

#[derive(Debug, Deserialize, Clone)]
struct TgMessage {
    chat: TgChat,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct TgChat {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct TgCallbackQuery {
    id: String,
    #[serde(default)]
    data: Option<String>,
    #[serde(default)]
    message: Option<TgMessage>,
}

pub async fn start(pool: SqlitePool, cfg: Arc<AppConfig>) -> Result<mpsc::Sender<TgEvent>> {
    if !cfg.telegram.enabled {
        anyhow::bail!("telegram 未启用");
    }
    if cfg.telegram.bot_token.trim().is_empty() {
        anyhow::bail!("telegram.bot_token 为空");
    }

    let client = Client::builder()
        .connect_timeout(Duration::from_secs(
            cfg.telegram.request_timeout_secs.max(3),
        ))
        .build()
        .context("构建 Telegram HTTP 客户端失败")?;
    let offset = parse_timezone(&cfg.telegram.timezone).unwrap_or_else(|| {
        warn!(
            timezone = %cfg.telegram.timezone,
            "无法解析时区，回落到 +08:00。支持 ±HH:MM 偏移和无 DST 的 IANA 别名（Asia/Shanghai/Tokyo、Australia/Brisbane 等）；DST 时区（Europe/London、America/* 等）请用显式偏移如 +00:00 / -05:00"
        );
        FixedOffset::east_opt(8 * 3600).expect("8h 偏移恒定有效")
    });
    let ctx = TgContext {
        pool,
        cfg,
        client,
        offset,
        pending_inputs: Arc::new(Mutex::new(HashMap::new())),
    };
    ensure_admin_defaults(&ctx).await?;

    let (tx, mut rx) = mpsc::channel::<TgEvent>(64);

    {
        let ctx = ctx.clone();
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    TgEvent::QuotaAlert { username, percent } => {
                        if let Err(e) = handle_quota_alert(&ctx, &username, percent).await {
                            warn!("Telegram 阈值通知失败: {}", e);
                        }
                    }
                }
            }
        });
    }

    {
        let ctx = ctx.clone();
        tokio::spawn(async move {
            poll_updates_loop(ctx).await;
        });
    }

    {
        let ctx = ctx.clone();
        tokio::spawn(async move {
            schedule_loop(ctx).await;
        });
    }

    Ok(tx)
}

async fn ensure_admin_defaults(ctx: &TgContext) -> Result<()> {
    let times = normalized_schedule_list(&ctx.cfg.telegram.admin_schedule_times)
        .unwrap_or_else(default_schedule_times_json);
    for chat_id in &ctx.cfg.telegram.admin_chat_ids {
        tg_repo::ensure_admin_pref(
            &ctx.pool,
            *chat_id,
            ctx.cfg.telegram.admin_notify_quota,
            ctx.cfg.telegram.admin_schedule_enabled,
            &times,
        )
        .await?;
    }
    Ok(())
}

async fn poll_updates_loop(ctx: TgContext) {
    let mut offset = 0i64;
    let mut backoff_secs = 1u64;
    let normal_secs = ctx.cfg.telegram.poll_interval_secs.max(1);
    loop {
        match get_updates(&ctx, offset).await {
            Ok(updates) => {
                backoff_secs = 1;
                for update in updates {
                    offset = update.update_id + 1;
                    if let Err(e) = handle_update(&ctx, update).await {
                        warn!("处理 Telegram update 失败: {}", e);
                    }
                }
                tokio::time::sleep(Duration::from_secs(normal_secs)).await;
            }
            Err(e) => {
                warn!(
                    error = %e,
                    backoff_secs,
                    "Telegram getUpdates 失败，将退避后重试"
                );
                tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
                backoff_secs = (backoff_secs * 2).min(60);
            }
        }
    }
}

async fn get_updates(ctx: &TgContext, offset: i64) -> Result<Vec<TgUpdate>> {
    let payload = json!({
        "offset": offset,
        "timeout": 25,
        "allowed_updates": ["message", "callback_query"],
    });
    let url = api_url(&ctx.cfg.telegram.bot_token, "getUpdates");
    let resp = ctx
        .client
        .post(url)
        .timeout(Duration::from_secs(35))
        .json(&payload)
        .send()
        .await
        .context("请求 Telegram getUpdates 失败")?;
    let data: TgResponse<Vec<TgUpdate>> = resp.json().await.context("解析 getUpdates 失败")?;
    if !data.ok {
        anyhow::bail!("Telegram getUpdates 返回 ok=false");
    }
    Ok(data.result)
}

async fn handle_update(ctx: &TgContext, update: TgUpdate) -> Result<()> {
    if let Some(message) = update.message {
        if let Some(text) = message.text {
            handle_message(ctx, message.chat.id, text.trim()).await?;
        }
    }
    if let Some(cb) = update.callback_query {
        if let Some(data) = cb.data.clone() {
            if let Some(msg) = cb.message.clone() {
                handle_callback(ctx, msg.chat.id, &data).await?;
            }
        }
        let _ = answer_callback(ctx, &cb.id).await;
    }
    Ok(())
}

async fn handle_message(ctx: &TgContext, chat_id: i64, text: &str) -> Result<()> {
    if text.starts_with('/') {
        ctx.pending_inputs.lock().await.remove(&chat_id);
    } else if let Some(pending) = ctx.pending_inputs.lock().await.get(&chat_id).cloned() {
        return handle_pending_input(ctx, chat_id, text, pending).await;
    }

    if let Some(code) = text.strip_prefix("/bind ").map(str::trim) {
        return match bind_user(ctx, chat_id, code).await {
            Ok(()) => Ok(()),
            Err(e) => send_text(ctx, chat_id, &e.to_string(), None).await,
        };
    }
    match text {
        "/start" => send_start(ctx, chat_id).await,
        "/usage" => match send_usage(ctx, chat_id).await {
            Ok(()) => Ok(()),
            Err(e) => send_text(ctx, chat_id, &e.to_string(), None).await,
        },
        "/usages" => match send_all_usages(ctx, chat_id).await {
            Ok(()) => Ok(()),
            Err(e) => send_text(ctx, chat_id, &e.to_string(), None).await,
        },
        _ => send_text(ctx, chat_id, "可用命令：/start /bind <绑定码> /usage", None).await,
    }
}

async fn handle_pending_input(
    ctx: &TgContext,
    chat_id: i64,
    text: &str,
    pending: PendingInput,
) -> Result<()> {
    let times = match parse_schedule_input(text) {
        Ok(times) => times,
        Err(e) => {
            send_text(
                ctx,
                chat_id,
                &format!("时间格式无效: {}\n请输入 HH:MM,HH:MM，例如 09:00,21:30", e),
                None,
            )
            .await?;
            return Ok(());
        }
    };
    let json = serde_json::to_string(&times)?;
    match pending {
        PendingInput::UserSchedule { username } => {
            let user = user_repo::get(&ctx.pool, &username)
                .await?
                .ok_or_else(|| anyhow!("用户不存在: {}", username))?;
            user_repo::set_tg_notify_settings(
                &ctx.pool,
                &username,
                user.tg_notify_quota_80,
                user.tg_notify_quota_90,
                user.tg_notify_quota_100,
                user.tg_schedule_enabled,
                &json,
            )
            .await?;
            send_text(
                ctx,
                chat_id,
                &format!("已更新定时推送时间：{}", times.join(", ")),
                Some(user_settings_keyboard()),
            )
            .await?;
        }
        PendingInput::AdminSchedule => {
            let prefs = load_admin_pref(ctx, chat_id).await?;
            tg_repo::set_admin_schedule(&ctx.pool, chat_id, prefs.schedule_enabled, &json).await?;
            send_text(
                ctx,
                chat_id,
                &format!("已更新管理员汇总时间：{}", times.join(", ")),
                Some(admin_settings_keyboard()),
            )
            .await?;
        }
    }
    ctx.pending_inputs.lock().await.remove(&chat_id);
    Ok(())
}

async fn handle_callback(ctx: &TgContext, chat_id: i64, data: &str) -> Result<()> {
    let result = match data {
        "home" => send_start(ctx, chat_id).await,
        "user:usage" => send_usage(ctx, chat_id).await,
        "user:refresh" => refresh_and_send_usage(ctx, chat_id, None).await,
        "user:subs" => send_subscription_menu(ctx, chat_id, None).await,
        "user:sub:url" => send_subscription_url(ctx, chat_id, None).await,
        "user:sub:b64" => send_subscription_base64(ctx, chat_id, None).await,
        "user:sub:plain" => send_subscription_plain(ctx, chat_id, None).await,
        "user:settings" => send_user_settings(ctx, chat_id).await,
        "user:set:n80" => toggle_user_setting(ctx, chat_id, 80).await,
        "user:set:n90" => toggle_user_setting(ctx, chat_id, 90).await,
        "user:set:n100" => toggle_user_setting(ctx, chat_id, 100).await,
        "user:set:schedule" => toggle_user_schedule(ctx, chat_id).await,
        "user:set:times" => prompt_user_times(ctx, chat_id).await,
        "admin:home" => send_admin_home(ctx, chat_id).await,
        "admin:usages" => send_all_usages(ctx, chat_id).await,
        "admin:users" => send_user_picker(ctx, chat_id).await,
        "admin:settings" => send_admin_settings(ctx, chat_id).await,
        "admin:set:quota" => toggle_admin_quota(ctx, chat_id).await,
        "admin:set:schedule" => toggle_admin_schedule(ctx, chat_id).await,
        "admin:set:times" => prompt_admin_times(ctx, chat_id).await,
        _ if data.starts_with("admin:user:") => {
            let name = data.trim_start_matches("admin:user:");
            send_admin_user_card(ctx, chat_id, name).await
        }
        _ if data.starts_with("admin:uusage:") => {
            let name = data.trim_start_matches("admin:uusage:");
            send_admin_user_usage(ctx, chat_id, name).await
        }
        _ if data.starts_with("admin:urefresh:") => {
            let name = data.trim_start_matches("admin:urefresh:");
            refresh_and_send_usage(ctx, chat_id, Some(name)).await
        }
        _ if data.starts_with("admin:usubs:") => {
            let name = data.trim_start_matches("admin:usubs:");
            send_subscription_menu(ctx, chat_id, Some(name)).await
        }
        _ if data.starts_with("admin:sub:url:") => {
            let name = data.trim_start_matches("admin:sub:url:");
            send_subscription_url(ctx, chat_id, Some(name)).await
        }
        _ if data.starts_with("admin:sub:b64:") => {
            let name = data.trim_start_matches("admin:sub:b64:");
            send_subscription_base64(ctx, chat_id, Some(name)).await
        }
        _ if data.starts_with("admin:sub:plain:") => {
            let name = data.trim_start_matches("admin:sub:plain:");
            send_subscription_plain(ctx, chat_id, Some(name)).await
        }
        // 注意：更具体的 prefix（admin:bind:regen:/admin:bind:unbind:）必须放在
        // admin:bind: 之前，否则会被它先吞掉。
        _ if data.starts_with("admin:bind:regen:") => {
            let name = data.trim_start_matches("admin:bind:regen:");
            admin_bind_regen(ctx, chat_id, name).await
        }
        _ if data.starts_with("admin:bind:unbind:") => {
            let name = data.trim_start_matches("admin:bind:unbind:");
            admin_bind_unbind(ctx, chat_id, name).await
        }
        _ if data.starts_with("admin:bind:") => {
            let name = data.trim_start_matches("admin:bind:");
            send_admin_bind_card(ctx, chat_id, name).await
        }
        _ => Ok(()),
    };
    if let Err(e) = result {
        let _ = send_text(ctx, chat_id, &e.to_string(), None).await;
    }
    Ok(())
}

async fn bind_user(ctx: &TgContext, chat_id: i64, code: &str) -> Result<()> {
    let user = user_repo::find_by_tg_bind_token(&ctx.pool, code)
        .await?
        .ok_or_else(|| anyhow!("绑定码无效"))?;
    user_repo::clear_tg_binding_for_chat(&ctx.pool, chat_id).await?;
    user_repo::set_tg_binding(&ctx.pool, &user.name, chat_id).await?;
    send_text(
        ctx,
        chat_id,
        &format!("绑定成功\n账号：{}", user.name),
        Some(start_keyboard(true, is_admin(ctx, chat_id))),
    )
    .await
}

async fn send_start(ctx: &TgContext, chat_id: i64) -> Result<()> {
    let bound = user_repo::find_by_tg_chat_id(&ctx.pool, chat_id).await?;
    let admin = is_admin(ctx, chat_id);
    match bound {
        Some(user) => {
            let text = user_home_text(&user);
            send_html(ctx, chat_id, &text, Some(start_keyboard(true, admin))).await
        }
        None if admin => send_admin_home(ctx, chat_id).await,
        None => {
            send_text(
                ctx,
                chat_id,
                "你还没有绑定账号。\n\n请先发送：\n/bind <绑定码>",
                None,
            )
            .await
        }
    }
}

async fn send_usage(ctx: &TgContext, chat_id: i64) -> Result<()> {
    let user = bound_user(ctx, chat_id).await?;
    send_html(
        ctx,
        chat_id,
        &user_usage_text(&user, false),
        Some(user_usage_keyboard()),
    )
    .await
}

async fn send_all_usages(ctx: &TgContext, chat_id: i64) -> Result<()> {
    if !is_admin(ctx, chat_id) {
        anyhow::bail!("仅管理员可查看全部用户流量");
    }
    let users = user_repo::list_all(&ctx.pool).await?;
    send_html(
        ctx,
        chat_id,
        &all_usages_text(&users),
        Some(admin_overview_keyboard()),
    )
    .await
}

async fn send_admin_home(ctx: &TgContext, chat_id: i64) -> Result<()> {
    if !is_admin(ctx, chat_id) {
        anyhow::bail!("仅管理员可访问管理面板");
    }
    let users = user_repo::list_all(&ctx.pool).await?;
    let enabled = users.iter().filter(|u| u.enabled).count();
    let over = users.iter().filter(|u| u.is_over_quota()).count();
    let exp = users.iter().filter(|u| u.is_expired()).count();
    let text = format!(
        "管理员面板\n\n用户数：{}\n启用：{}\n超额：{}\n到期：{}",
        users.len(),
        enabled,
        over,
        exp
    );
    send_text(ctx, chat_id, &text, Some(admin_home_keyboard())).await
}

async fn send_user_picker(ctx: &TgContext, chat_id: i64) -> Result<()> {
    if !is_admin(ctx, chat_id) {
        anyhow::bail!("仅管理员可查看用户列表");
    }
    let users = user_repo::list_all(&ctx.pool).await?;
    let mut rows = Vec::new();
    for u in users {
        rows.push(vec![(u.name.clone(), format!("admin:user:{}", u.name))]);
    }
    rows.push(vec![("返回管理员首页".into(), "admin:home".into())]);
    send_text(ctx, chat_id, "选择用户", Some(inline_keyboard(rows))).await
}

async fn send_admin_user_card(ctx: &TgContext, chat_id: i64, username: &str) -> Result<()> {
    if !is_admin(ctx, chat_id) {
        anyhow::bail!("仅管理员可查看用户");
    }
    let user = user_repo::get(&ctx.pool, username)
        .await?
        .ok_or_else(|| anyhow!("用户不存在: {}", username))?;
    let text = admin_user_card_text(&user);
    let rows = vec![
        vec![
            ("查看流量".into(), format!("admin:uusage:{}", user.name)),
            ("查看订阅".into(), format!("admin:usubs:{}", user.name)),
        ],
        vec![
            ("TG 绑定".into(), format!("admin:bind:{}", user.name)),
            ("刷新流量".into(), format!("admin:urefresh:{}", user.name)),
        ],
        vec![("返回用户列表".into(), "admin:users".into())],
    ];
    send_html(ctx, chat_id, &text, Some(inline_keyboard(rows))).await
}

async fn send_admin_user_usage(ctx: &TgContext, chat_id: i64, username: &str) -> Result<()> {
    if !is_admin(ctx, chat_id) {
        anyhow::bail!("仅管理员可查看用户流量");
    }
    let user = user_repo::get(&ctx.pool, username)
        .await?
        .ok_or_else(|| anyhow!("用户不存在: {}", username))?;
    let rows = vec![
        vec![
            ("刷新流量".into(), format!("admin:urefresh:{}", user.name)),
            ("返回用户卡片".into(), format!("admin:user:{}", user.name)),
        ],
        vec![("返回用户列表".into(), "admin:users".into())],
    ];
    send_html(
        ctx,
        chat_id,
        &user_usage_text(&user, true),
        Some(inline_keyboard(rows)),
    )
    .await
}

async fn send_admin_bind_card(ctx: &TgContext, chat_id: i64, username: &str) -> Result<()> {
    if !is_admin(ctx, chat_id) {
        anyhow::bail!("仅管理员可管理 TG 绑定");
    }
    let user = user_repo::get(&ctx.pool, username)
        .await?
        .ok_or_else(|| anyhow!("用户不存在: {}", username))?;
    let bound = user.tg_is_bound();
    let body = if bound {
        format!(
            "🔗 TG 绑定管理 · <b>{name}</b>\n\n\
             状态:  🟢 已绑定\n\
             chat_id:  <code>{chat_id_v}</code>\n\n\
             解绑后该用户在 TG 里发 <code>/bind &lt;新绑定码&gt;</code> 才能重新绑回。\n\
             重置绑定码会立即失效旧码。",
            name = h(&user.name),
            chat_id_v = user.tg_chat_id,
        )
    } else if user.tg_bind_token.is_empty() {
        format!(
            "🔗 TG 绑定管理 · <b>{name}</b>\n\n\
             状态:  ⚪ 未绑定\n\
             绑定码:  <i>未生成</i>\n\n\
             先「重置绑定码」生成一个，再让该用户在 TG 里发 <code>/bind &lt;码&gt;</code>。",
            name = h(&user.name),
        )
    } else {
        format!(
            "🔗 TG 绑定管理 · <b>{name}</b>\n\n\
             状态:  ⚪ 未绑定\n\
             绑定码:  <code>{token}</code>\n\n\
             让该用户在 TG 里发：<code>/bind {token}</code>",
            name = h(&user.name),
            token = h(&user.tg_bind_token),
        )
    };

    let mut rows: Vec<Vec<(String, String)>> = Vec::new();
    if bound {
        rows.push(vec![(
            "解绑当前账号".into(),
            format!("admin:bind:unbind:{}", user.name),
        )]);
    }
    rows.push(vec![(
        "重置绑定码".into(),
        format!("admin:bind:regen:{}", user.name),
    )]);
    rows.push(vec![
        ("返回用户卡片".into(), format!("admin:user:{}", user.name)),
        ("返回用户列表".into(), "admin:users".into()),
    ]);

    send_html(ctx, chat_id, &body, Some(inline_keyboard(rows))).await
}

async fn admin_bind_regen(ctx: &TgContext, chat_id: i64, username: &str) -> Result<()> {
    if !is_admin(ctx, chat_id) {
        anyhow::bail!("仅管理员可管理 TG 绑定");
    }
    user_repo::get(&ctx.pool, username)
        .await?
        .ok_or_else(|| anyhow!("用户不存在: {}", username))?;
    let new_token = crate::service::user_service::new_tg_bind_token();
    user_repo::set_tg_bind_token(&ctx.pool, username, &new_token).await?;
    send_admin_bind_card(ctx, chat_id, username).await
}

async fn admin_bind_unbind(ctx: &TgContext, chat_id: i64, username: &str) -> Result<()> {
    if !is_admin(ctx, chat_id) {
        anyhow::bail!("仅管理员可管理 TG 绑定");
    }
    let user = user_repo::get(&ctx.pool, username)
        .await?
        .ok_or_else(|| anyhow!("用户不存在: {}", username))?;
    if user.tg_is_bound() {
        // chat_id=0 在 set_tg_binding 里其实可以接受，但 user 端"绑定"语义统一用
        // user_repo::set_tg_binding(name, 0) 来标识"已解绑"。
        user_repo::set_tg_binding(&ctx.pool, username, 0).await?;
    }
    send_admin_bind_card(ctx, chat_id, username).await
}

async fn send_subscription_menu(ctx: &TgContext, chat_id: i64, target: Option<&str>) -> Result<()> {
    let target_name = resolve_target_user(ctx, chat_id, target).await?.name;
    let rows = if target.is_some() {
        vec![
            vec![("URL 订阅".into(), format!("admin:sub:url:{}", target_name))],
            vec![(
                "Base64 订阅".into(),
                format!("admin:sub:b64:{}", target_name),
            )],
            vec![(
                "明文节点".into(),
                format!("admin:sub:plain:{}", target_name),
            )],
            vec![("返回用户卡片".into(), format!("admin:user:{}", target_name))],
        ]
    } else {
        vec![
            vec![("URL 订阅".into(), "user:sub:url".into())],
            vec![("Base64 订阅".into(), "user:sub:b64".into())],
            vec![("明文节点".into(), "user:sub:plain".into())],
            vec![("返回首页".into(), "home".into())],
        ]
    };
    send_text(ctx, chat_id, "请选择订阅内容", Some(inline_keyboard(rows))).await
}

async fn send_subscription_url(ctx: &TgContext, chat_id: i64, target: Option<&str>) -> Result<()> {
    let export = build_export_payloads(ctx, chat_id, target).await?;
    let text = match export.url {
        Some(url) => format!("URL 订阅\n\n{}", url),
        None => "当前未启用 URL 订阅。".into(),
    };
    send_text(
        ctx,
        chat_id,
        &text,
        Some(subscription_back_keyboard(target)),
    )
    .await
}

async fn send_subscription_base64(
    ctx: &TgContext,
    chat_id: i64,
    target: Option<&str>,
) -> Result<()> {
    let export = build_export_payloads(ctx, chat_id, target).await?;
    send_long_text(
        ctx,
        chat_id,
        &format!("Base64 订阅\n\n{}", export.base64),
        Some(subscription_back_keyboard(target)),
    )
    .await
}

async fn send_subscription_plain(
    ctx: &TgContext,
    chat_id: i64,
    target: Option<&str>,
) -> Result<()> {
    let export = build_export_payloads(ctx, chat_id, target).await?;
    let plain = if export.plain_links.is_empty() {
        "无可用节点".to_string()
    } else {
        export.plain_links.join("\n")
    };
    send_long_text(
        ctx,
        chat_id,
        &format!("明文节点\n\n{}", plain),
        Some(subscription_back_keyboard(target)),
    )
    .await
}

async fn send_user_settings(ctx: &TgContext, chat_id: i64) -> Result<()> {
    let user = bound_user(ctx, chat_id).await?;
    let times = effective_user_schedule_times(ctx, &user);
    let text = format!(
        "通知设置\n时区：{}\n\n80% 阈值：{}\n90% 阈值：{}\n100% 阈值：{}\n定时推送：{}\n推送时间：{}",
        ctx.cfg.telegram.timezone,
        on_off(user.tg_notify_quota_80),
        on_off(user.tg_notify_quota_90),
        on_off(user.tg_notify_quota_100),
        on_off(user.tg_schedule_enabled),
        if times.is_empty() {
            "未设置".into()
        } else {
            times.join(", ")
        }
    );
    send_text(ctx, chat_id, &text, Some(user_settings_keyboard())).await
}

async fn toggle_user_setting(ctx: &TgContext, chat_id: i64, level: u8) -> Result<()> {
    let user = bound_user(ctx, chat_id).await?;
    let (n80, n90, n100) = match level {
        80 => (
            !user.tg_notify_quota_80,
            user.tg_notify_quota_90,
            user.tg_notify_quota_100,
        ),
        90 => (
            user.tg_notify_quota_80,
            !user.tg_notify_quota_90,
            user.tg_notify_quota_100,
        ),
        100 => (
            user.tg_notify_quota_80,
            user.tg_notify_quota_90,
            !user.tg_notify_quota_100,
        ),
        _ => (
            user.tg_notify_quota_80,
            user.tg_notify_quota_90,
            user.tg_notify_quota_100,
        ),
    };
    user_repo::set_tg_notify_settings(
        &ctx.pool,
        &user.name,
        n80,
        n90,
        n100,
        user.tg_schedule_enabled,
        &user.tg_schedule_times,
    )
    .await?;
    send_user_settings(ctx, chat_id).await
}

async fn toggle_user_schedule(ctx: &TgContext, chat_id: i64) -> Result<()> {
    let user = bound_user(ctx, chat_id).await?;
    user_repo::set_tg_notify_settings(
        &ctx.pool,
        &user.name,
        user.tg_notify_quota_80,
        user.tg_notify_quota_90,
        user.tg_notify_quota_100,
        !user.tg_schedule_enabled,
        &user.tg_schedule_times,
    )
    .await?;
    send_user_settings(ctx, chat_id).await
}

async fn prompt_user_times(ctx: &TgContext, chat_id: i64) -> Result<()> {
    let user = bound_user(ctx, chat_id).await?;
    ctx.pending_inputs.lock().await.insert(
        chat_id,
        PendingInput::UserSchedule {
            username: user.name,
        },
    );
    send_text(
        ctx,
        chat_id,
        &format!(
            "请输入定时推送时间，支持多个。\n格式：HH:MM,HH:MM\n示例：09:00,21:30\n\n时区：{}",
            timezone_label(ctx)
        ),
        None,
    )
    .await
}

async fn send_admin_settings(ctx: &TgContext, chat_id: i64) -> Result<()> {
    if !is_admin(ctx, chat_id) {
        anyhow::bail!("仅管理员可配置管理员通知");
    }
    let prefs = load_admin_pref(ctx, chat_id).await?;
    let times = effective_admin_schedule_times(ctx, &prefs);
    let text = format!(
        "管理员通知设置\n时区：{}\n\n阈值警告：{}\n定时汇总：{}\n汇总时间：{}",
        ctx.cfg.telegram.timezone,
        on_off(prefs.notify_quota),
        on_off(prefs.schedule_enabled),
        if times.is_empty() {
            "未设置".into()
        } else {
            times.join(", ")
        }
    );
    send_text(ctx, chat_id, &text, Some(admin_settings_keyboard())).await
}

async fn toggle_admin_quota(ctx: &TgContext, chat_id: i64) -> Result<()> {
    if !is_admin(ctx, chat_id) {
        anyhow::bail!("仅管理员可配置管理员通知");
    }
    let prefs = load_admin_pref(ctx, chat_id).await?;
    tg_repo::set_admin_notify_quota(&ctx.pool, chat_id, !prefs.notify_quota).await?;
    send_admin_settings(ctx, chat_id).await
}

async fn toggle_admin_schedule(ctx: &TgContext, chat_id: i64) -> Result<()> {
    if !is_admin(ctx, chat_id) {
        anyhow::bail!("仅管理员可配置管理员通知");
    }
    let prefs = load_admin_pref(ctx, chat_id).await?;
    tg_repo::set_admin_schedule(
        &ctx.pool,
        chat_id,
        !prefs.schedule_enabled,
        &prefs.schedule_times,
    )
    .await?;
    send_admin_settings(ctx, chat_id).await
}

async fn prompt_admin_times(ctx: &TgContext, chat_id: i64) -> Result<()> {
    if !is_admin(ctx, chat_id) {
        anyhow::bail!("仅管理员可配置管理员通知");
    }
    ctx.pending_inputs
        .lock()
        .await
        .insert(chat_id, PendingInput::AdminSchedule);
    send_text(
        ctx,
        chat_id,
        &format!(
            "请输入管理员汇总时间，支持多个。\n格式：HH:MM,HH:MM\n示例：09:00,13:00,21:30\n\n时区：{}",
            timezone_label(ctx)
        ),
        None,
    )
    .await
}

async fn refresh_and_send_usage(ctx: &TgContext, chat_id: i64, target: Option<&str>) -> Result<()> {
    let username = resolve_target_user(ctx, chat_id, target).await?.name;
    let flush_msg = match crate::service::runtime_service::flush_current_traffic(
        &ctx.pool,
        &ctx.cfg.singbox.grpc_addr,
    )
    .await
    {
        Ok(_) => "✓ 已刷新当前流量\n\n".to_string(),
        Err(e) => format!("⚠️ 刷新失败（显示缓存数据）: {}\n\n", h(&e.to_string())),
    };
    let user = user_repo::get(&ctx.pool, &username)
        .await?
        .ok_or_else(|| anyhow!("用户不存在: {}", username))?;
    let keyboard = if target.is_some() {
        inline_keyboard(vec![
            vec![
                ("查看流量".into(), format!("admin:uusage:{}", user.name)),
                ("返回用户卡片".into(), format!("admin:user:{}", user.name)),
            ],
            vec![("返回用户列表".into(), "admin:users".into())],
        ])
    } else {
        user_usage_keyboard()
    };
    send_html(
        ctx,
        chat_id,
        &format!("{}{}", flush_msg, user_usage_text(&user, target.is_some())),
        Some(keyboard),
    )
    .await
}

async fn schedule_loop(ctx: TgContext) {
    let mut iv = tokio::time::interval(Duration::from_secs(30));
    loop {
        iv.tick().await;
        if let Err(e) = normalize_quota_levels(&ctx).await {
            warn!("刷新 Telegram 阈值状态失败: {}", e);
        }
        if let Err(e) = run_due_schedules(&ctx).await {
            warn!("执行 Telegram 定时通知失败: {}", e);
        }
    }
}

async fn normalize_quota_levels(ctx: &TgContext) -> Result<()> {
    let users = user_repo::list_all(&ctx.pool).await?;
    for user in users {
        let current = quota_level(user.quota_used_percent());
        if i64::from(current) < user.tg_last_quota_level {
            user_repo::set_tg_last_quota_level(&ctx.pool, &user.name, i64::from(current)).await?;
        }
    }
    Ok(())
}

async fn run_due_schedules(ctx: &TgContext) -> Result<()> {
    let now = local_now(ctx);
    let today = now.format("%Y-%m-%d").to_string();

    let users = user_repo::list_all(&ctx.pool).await?;
    for user in users.into_iter().filter(|u| u.tg_is_bound()) {
        if !user.tg_schedule_enabled {
            continue;
        }
        let times = effective_user_schedule_times(ctx, &user);
        if times.is_empty() {
            continue;
        }
        let mut dates = user.tg_last_schedule_dates();
        let due = due_times(&now, &times, &dates);
        if due.is_empty() {
            continue;
        }
        let text = format!(
            "⏰ 定时流量播报\n时间: {}\n\n{}",
            h(&now.format("%Y-%m-%d %H:%M").to_string()),
            user_usage_text(&user, false)
        );
        send_html(ctx, user.tg_chat_id, &text, Some(user_usage_keyboard())).await?;
        for item in due {
            dates.insert(item, today.clone());
        }
        user_repo::set_tg_last_schedule_dates(
            &ctx.pool,
            &user.name,
            &serde_json::to_string(&dates)?,
        )
        .await?;
    }

    let admins = tg_repo::list_admin_prefs(&ctx.pool, &ctx.cfg.telegram.admin_chat_ids).await?;
    for prefs in admins {
        if !prefs.schedule_enabled {
            continue;
        }
        let times = effective_admin_schedule_times(ctx, &prefs);
        if times.is_empty() {
            continue;
        }
        let mut dates = prefs.last_schedule_dates();
        let due = due_times(&now, &times, &dates);
        if due.is_empty() {
            continue;
        }
        let users = user_repo::list_all(&ctx.pool).await?;
        let text = format!(
            "⏰ 定时流量汇总\n时间: {}\n\n{}",
            h(&now.format("%Y-%m-%d %H:%M").to_string()),
            all_usages_text(&users)
        );
        send_html(ctx, prefs.chat_id, &text, Some(admin_overview_keyboard())).await?;
        for item in due {
            dates.insert(item, today.clone());
        }
        tg_repo::set_admin_last_schedule_dates(
            &ctx.pool,
            prefs.chat_id,
            &serde_json::to_string(&dates)?,
        )
        .await?;
    }

    Ok(())
}

async fn handle_quota_alert(ctx: &TgContext, username: &str, percent: u8) -> Result<()> {
    let level = quota_level(percent as f64);
    if level == 0 {
        return Ok(());
    }
    let user = user_repo::get(&ctx.pool, username)
        .await?
        .ok_or_else(|| anyhow!("用户不存在: {}", username))?;
    if i64::from(level) <= user.tg_last_quota_level {
        return Ok(());
    }

    if user.tg_is_bound() && user_threshold_enabled(&user, level) {
        let pct_f = user.quota_used_percent();
        let text = format!(
            "{emoji} 流量提醒\n\n\
             账号:  <b>{name}</b>\n\
             套餐:  <b>{quota}</b>（{billing}）\n\
             已用:  <b>{used}</b> / {total} ({pct:.0}%)\n\
             剩余:  <b>{remain}</b>\n\
             进度:  <code>{bar} {pct_f:.1}%</code>\n\
             重置:  {reset}",
            emoji = quota_alert_emoji(level),
            name = h(&user.name),
            quota = h(&quota_label(user.quota_gb)),
            billing = h(&billing_label(user.traffic_multiplier)),
            used = h(&User::format_bytes(user.used_total_bytes())),
            total = h(&quota_label(user.quota_gb)),
            pct = percent,
            remain = h(&remaining_label(&user)),
            bar = progress_bar(pct_f),
            pct_f = pct_f,
            reset = h(&reset_label(user.reset_day)),
        );
        send_html(ctx, user.tg_chat_id, &text, Some(user_alert_keyboard())).await?;
    }

    let admins = tg_repo::list_admin_prefs(&ctx.pool, &ctx.cfg.telegram.admin_chat_ids).await?;
    for admin in admins.into_iter().filter(|a| a.notify_quota) {
        let text = format!(
            "{emoji} 用户流量提醒\n\n\
             <b>{name}</b> 达到 <b>{level}%</b>\n\
             已用:  {used} / {quota}\n\
             剩余:  {remain}",
            emoji = quota_alert_emoji(level),
            name = h(&user.name),
            level = level,
            used = h(&User::format_bytes(user.used_total_bytes())),
            quota = h(&quota_label(user.quota_gb)),
            remain = h(&remaining_label(&user)),
        );
        send_html(ctx, admin.chat_id, &text, Some(admin_overview_keyboard())).await?;
    }

    user_repo::set_tg_last_quota_level(&ctx.pool, &user.name, i64::from(level)).await?;
    Ok(())
}

async fn build_export_payloads(
    ctx: &TgContext,
    chat_id: i64,
    target: Option<&str>,
) -> Result<ExportPayload> {
    let user = resolve_target_user(ctx, chat_id, target).await?;
    let cfg_json = crate::core::config::load(&ctx.cfg.singbox.config_path)?;
    let server = crate::service::node_service::resolve_export_server(
        ctx.cfg.subscription.use_public_base_as_server,
        &ctx.cfg.subscription.public_base,
        None,
    )
    .await?;
    let links = crate::service::sub_service::generate_links(&cfg_json, &user.name, &server)?;
    let plain_links = links
        .iter()
        .map(|item| format!("[{}] {}", item.protocol, item.link))
        .collect::<Vec<_>>();
    let base64 = crate::service::sub_service::generate_subscription(&links);
    let url = if !ctx.cfg.subscription.public_base.trim().is_empty() && !user.sub_token.is_empty() {
        Some(format!(
            "{}/sub/{}",
            ctx.cfg.subscription.public_base.trim_end_matches('/'),
            user.sub_token
        ))
    } else {
        None
    };
    Ok(ExportPayload {
        base64,
        plain_links,
        url,
    })
}

struct ExportPayload {
    url: Option<String>,
    base64: String,
    plain_links: Vec<String>,
}

async fn load_admin_pref(ctx: &TgContext, chat_id: i64) -> Result<TgAdminPrefs> {
    tg_repo::get_admin_pref(&ctx.pool, chat_id)
        .await?
        .ok_or_else(|| anyhow!("管理员未初始化: {}", chat_id))
}

async fn resolve_target_user(ctx: &TgContext, chat_id: i64, target: Option<&str>) -> Result<User> {
    match target {
        Some(name) => {
            if !is_admin(ctx, chat_id) {
                anyhow::bail!("仅管理员可查看其他用户");
            }
            user_repo::get(&ctx.pool, name)
                .await?
                .ok_or_else(|| anyhow!("用户不存在: {}", name))
        }
        None => bound_user(ctx, chat_id).await,
    }
}

async fn bound_user(ctx: &TgContext, chat_id: i64) -> Result<User> {
    user_repo::find_by_tg_chat_id(&ctx.pool, chat_id)
        .await?
        .ok_or_else(|| anyhow!("当前 TG 账号未绑定用户，请先 /bind <绑定码>"))
}

fn is_admin(ctx: &TgContext, chat_id: i64) -> bool {
    ctx.cfg.telegram.admin_chat_ids.contains(&chat_id)
}

/// 给 TG 提示文案用的时区标签：原样回显配置值；空串显示默认（+08:00）。
/// 注意：用户在 config.toml 里填了无效字符串（如 `Europe/London`）时，此处仍
/// 显示原值，但实际生效偏移已经是 fallback 的 +08:00（见 parse_timezone 的 warn）。
/// 这是有意为之——管理员从 systemd 日志里看到 warn 才会去改配置，UI 文案
/// 与 config 字面量一致便于排查。
fn timezone_label(ctx: &TgContext) -> String {
    let s = ctx.cfg.telegram.timezone.trim();
    if s.is_empty() {
        "Asia/Shanghai（默认）".into()
    } else {
        s.to_string()
    }
}

fn local_now(ctx: &TgContext) -> DateTime<FixedOffset> {
    Utc::now().with_timezone(&ctx.offset)
}

fn quota_level(percent: f64) -> u8 {
    if percent >= 100.0 {
        100
    } else if percent >= 90.0 {
        90
    } else if percent >= 80.0 {
        80
    } else {
        0
    }
}

fn user_threshold_enabled(user: &User, level: u8) -> bool {
    match level {
        80 => user.tg_notify_quota_80,
        90 => user.tg_notify_quota_90,
        100 => user.tg_notify_quota_100,
        _ => false,
    }
}

fn effective_user_schedule_times(ctx: &TgContext, user: &User) -> Vec<String> {
    let own = user.tg_schedule_times();
    if own.is_empty() {
        normalized_schedule_vec(&ctx.cfg.telegram.default_schedule_times)
    } else {
        normalized_schedule_vec(&own)
    }
}

fn effective_admin_schedule_times(ctx: &TgContext, prefs: &TgAdminPrefs) -> Vec<String> {
    let own = prefs.schedule_times();
    if own.is_empty() {
        normalized_schedule_vec(&ctx.cfg.telegram.admin_schedule_times)
    } else {
        normalized_schedule_vec(&own)
    }
}

fn due_times(
    now: &DateTime<FixedOffset>,
    times: &[String],
    dates: &BTreeMap<String, String>,
) -> Vec<String> {
    let today = now.format("%Y-%m-%d").to_string();
    times
        .iter()
        .filter_map(|item| {
            let (hh, mm) = parse_single_time(item)?;
            if now.hour() == hh && now.minute() == mm && dates.get(item) != Some(&today) {
                Some(item.clone())
            } else {
                None
            }
        })
        .collect()
}

fn on_off(v: bool) -> &'static str {
    if v {
        "开启"
    } else {
        "关闭"
    }
}

fn billing_label(multiplier: f64) -> String {
    if (multiplier - 2.0).abs() < 0.01 {
        "双向".into()
    } else if (multiplier - 1.0).abs() < 0.01 {
        "单向".into()
    } else {
        format!("{:.1}x", multiplier)
    }
}

fn quota_label(quota_gb: f64) -> String {
    if quota_gb <= 0.0 {
        "不限".into()
    } else {
        format!("{:.0}G", quota_gb)
    }
}

fn reset_label(reset_day: i64) -> String {
    match reset_day {
        0 => "不重置".into(),
        32 => "月末".into(),
        d => format!("每月{}日", d),
    }
}

fn remaining_label(user: &User) -> String {
    if user.quota_gb <= 0.0 {
        "不限".into()
    } else {
        User::format_bytes((user.quota_bytes() - user.used_total_bytes()).max(0))
    }
}

fn user_home_text(user: &User) -> String {
    let pct = user.quota_used_percent();
    let bar = progress_bar(pct);
    let expire = if user.expire_at.is_empty() {
        "永久".to_string()
    } else {
        h(&user.expire_at)
    };
    format!(
        "📊 我的账号\n\n\
         账号:  <b>{name}</b>\n\
         状态:  {se} {sw}\n\
         套餐:  <b>{quota}</b>（{billing}）\n\
         已用:  <b>{used}</b> / {total}\n\
         剩余:  <b>{remain}</b>\n\
         进度:  <code>{bar} {pct:.1}%</code>\n\
         重置:  {reset}\n\
         到期:  {expire}",
        name = h(&user.name),
        se = status_emoji(user.enabled),
        sw = if user.enabled { "启用" } else { "禁用" },
        quota = h(&quota_label(user.quota_gb)),
        billing = h(&billing_label(user.traffic_multiplier)),
        used = h(&User::format_bytes(user.used_total_bytes())),
        total = h(&quota_label(user.quota_gb)),
        remain = h(&remaining_label(user)),
        bar = bar,
        pct = pct,
        reset = h(&reset_label(user.reset_day)),
        expire = expire,
    )
}

fn user_usage_text(user: &User, admin_view: bool) -> String {
    let pct = user.quota_used_percent();
    let bar = progress_bar(pct);
    let title = if admin_view {
        format!("👤 用户 <b>{}</b> 流量", h(&user.name))
    } else {
        "📊 流量信息".to_string()
    };
    let expire = if user.expire_at.is_empty() {
        "永久".to_string()
    } else {
        h(&user.expire_at)
    };
    format!(
        "{title}\n\n\
         状态:  {se} {sw}\n\
         上行:  {up}\n\
         下行:  {down}\n\
         计费用量:  <b>{used}</b> / {total}\n\
         剩余:  <b>{remain}</b>\n\
         进度:  <code>{bar} {pct:.1}%</code>\n\
         重置:  {reset}\n\
         到期:  {expire}",
        title = title,
        se = status_emoji(user.enabled),
        sw = if user.enabled { "启用" } else { "禁用" },
        up = h(&User::format_bytes(user.used_up_bytes)),
        down = h(&User::format_bytes(user.used_down_bytes)),
        used = h(&User::format_bytes(user.used_total_bytes())),
        total = h(&quota_label(user.quota_gb)),
        remain = h(&remaining_label(user)),
        bar = bar,
        pct = pct,
        reset = h(&reset_label(user.reset_day)),
        expire = expire,
    )
}

fn admin_user_card_text(user: &User) -> String {
    let pct = user.quota_used_percent();
    let bar = progress_bar(pct);
    let bind_line = if user.tg_is_bound() {
        format!(
            "TG:    🟢 已绑定 (chat_id <code>{}</code>)",
            user.tg_chat_id
        )
    } else {
        "TG:    ⚪ 未绑定".to_string()
    };
    let expire = if user.expire_at.is_empty() {
        "永久".to_string()
    } else {
        h(&user.expire_at)
    };
    format!(
        "👤 用户 <b>{name}</b>\n\n\
         状态:  {se} {sw}\n\
         套餐:  <b>{quota}</b>（{billing}）\n\
         已用:  <b>{used}</b> / {total}\n\
         剩余:  <b>{remain}</b>\n\
         进度:  <code>{bar} {pct:.1}%</code>\n\
         重置:  {reset}\n\
         到期:  {expire}\n\
         {bind_line}",
        name = h(&user.name),
        se = status_emoji(user.enabled),
        sw = if user.enabled { "启用" } else { "禁用" },
        quota = h(&quota_label(user.quota_gb)),
        billing = h(&billing_label(user.traffic_multiplier)),
        used = h(&User::format_bytes(user.used_total_bytes())),
        total = h(&quota_label(user.quota_gb)),
        remain = h(&remaining_label(user)),
        bar = bar,
        pct = pct,
        reset = h(&reset_label(user.reset_day)),
        expire = expire,
        bind_line = bind_line,
    )
}

fn all_usages_text(users: &[User]) -> String {
    let mut rows = users.to_vec();
    rows.sort_by(|a, b| {
        b.used_total_bytes()
            .cmp(&a.used_total_bytes())
            .then_with(|| a.name.cmp(&b.name))
    });
    // 用 <pre> 包成等宽块，列对齐才不会被 Telegram 折叠空格。
    let mut body = String::with_capacity(rows.len() * 64);
    body.push_str("用户          已用       / 配额      百分比\n");
    body.push_str("──────────────────────────────────────\n");
    for user in &rows {
        body.push_str(&format!(
            "{:<12} {:>9} / {:<6} {:>6.1}%\n",
            // 限制 name 在 12 字符以内，避免一个超长用户名把整行撑出 Telegram 的宽度
            truncate_for_table(&user.name, 12),
            User::format_bytes(user.used_total_bytes()),
            quota_label(user.quota_gb),
            user.quota_used_percent()
        ));
    }
    format!("📋 全部用户流量\n\n<pre>{}</pre>", h(body.trim_end()))
}

/// 截断 ASCII 安全的窄宽截断；中文/emoji 因为是宽字符可能仍会撑列，
/// 但比之前完全不限至少能挡住极端情况。
fn truncate_for_table(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars - 1).collect();
    out.push('…');
    out
}

fn start_keyboard(bound: bool, admin: bool) -> Value {
    let mut rows: Vec<Vec<(String, String)>> = Vec::new();
    if bound {
        rows.push(vec![
            ("我的流量".into(), "user:usage".into()),
            ("我的订阅".into(), "user:subs".into()),
        ]);
        rows.push(vec![
            ("刷新流量".into(), "user:refresh".into()),
            ("通知设置".into(), "user:settings".into()),
        ]);
    }
    if admin {
        rows.push(vec![
            ("所有用户流量".into(), "admin:usages".into()),
            ("用户列表".into(), "admin:users".into()),
        ]);
        rows.push(vec![("管理员通知设置".into(), "admin:settings".into())]);
    }
    inline_keyboard(rows)
}

fn user_usage_keyboard() -> Value {
    inline_keyboard(vec![vec![
        ("刷新流量".into(), "user:refresh".into()),
        ("返回首页".into(), "home".into()),
    ]])
}

fn user_alert_keyboard() -> Value {
    inline_keyboard(vec![vec![
        ("我的流量".into(), "user:usage".into()),
        ("我的订阅".into(), "user:subs".into()),
    ]])
}

fn user_settings_keyboard() -> Value {
    inline_keyboard(vec![
        vec![
            ("切换80%".into(), "user:set:n80".into()),
            ("切换90%".into(), "user:set:n90".into()),
        ],
        vec![
            ("切换100%".into(), "user:set:n100".into()),
            ("切换定时".into(), "user:set:schedule".into()),
        ],
        vec![
            ("设置时间".into(), "user:set:times".into()),
            ("返回首页".into(), "home".into()),
        ],
    ])
}

fn admin_home_keyboard() -> Value {
    inline_keyboard(vec![
        vec![
            ("所有用户流量".into(), "admin:usages".into()),
            ("用户列表".into(), "admin:users".into()),
        ],
        vec![
            ("管理员通知设置".into(), "admin:settings".into()),
            ("返回首页".into(), "home".into()),
        ],
    ])
}

fn admin_overview_keyboard() -> Value {
    inline_keyboard(vec![
        vec![
            ("所有用户流量".into(), "admin:usages".into()),
            ("用户列表".into(), "admin:users".into()),
        ],
        vec![("返回管理员首页".into(), "admin:home".into())],
    ])
}

fn admin_settings_keyboard() -> Value {
    inline_keyboard(vec![
        vec![
            ("切换阈值警告".into(), "admin:set:quota".into()),
            ("切换定时汇总".into(), "admin:set:schedule".into()),
        ],
        vec![
            ("设置时间".into(), "admin:set:times".into()),
            ("返回管理员首页".into(), "admin:home".into()),
        ],
    ])
}

fn subscription_back_keyboard(target: Option<&str>) -> Value {
    match target {
        Some(name) => inline_keyboard(vec![
            vec![("返回订阅".into(), format!("admin:usubs:{}", name))],
            vec![("返回用户卡片".into(), format!("admin:user:{}", name))],
        ]),
        None => inline_keyboard(vec![
            vec![("返回订阅".into(), "user:subs".into())],
            vec![("返回首页".into(), "home".into())],
        ]),
    }
}

fn inline_keyboard(rows: Vec<Vec<(String, String)>>) -> Value {
    json!({
        "inline_keyboard": rows
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .map(|(text, data)| json!({"text": text, "callback_data": data}))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    })
}

async fn send_text(
    ctx: &TgContext,
    chat_id: i64,
    text: &str,
    reply_markup: Option<Value>,
) -> Result<()> {
    send_message(ctx, chat_id, text, reply_markup, false).await
}

/// HTML 模式发送：开启 parse_mode=HTML，调用方负责给所有用户输入做 HTML 转义。
async fn send_html(
    ctx: &TgContext,
    chat_id: i64,
    text: &str,
    reply_markup: Option<Value>,
) -> Result<()> {
    send_message(ctx, chat_id, text, reply_markup, true).await
}

async fn send_message(
    ctx: &TgContext,
    chat_id: i64,
    text: &str,
    reply_markup: Option<Value>,
    html: bool,
) -> Result<()> {
    let mut payload = json!({
        "chat_id": chat_id,
        "text": text,
        "disable_web_page_preview": true,
    });
    if html {
        payload["parse_mode"] = json!("HTML");
    }
    if let Some(markup) = reply_markup {
        payload["reply_markup"] = markup;
    }
    api_post(ctx, "sendMessage", &payload).await?;
    Ok(())
}

async fn send_long_text(
    ctx: &TgContext,
    chat_id: i64,
    text: &str,
    reply_markup: Option<Value>,
) -> Result<()> {
    let chunks = split_message(text, 3500);
    let last = chunks.len().saturating_sub(1);
    for (idx, chunk) in chunks.iter().enumerate() {
        let markup = if idx == last {
            reply_markup.clone()
        } else {
            None
        };
        send_text(ctx, chat_id, chunk, markup).await?;
    }
    Ok(())
}

async fn answer_callback(ctx: &TgContext, callback_id: &str) -> Result<()> {
    api_post(
        ctx,
        "answerCallbackQuery",
        &json!({
            "callback_query_id": callback_id,
        }),
    )
    .await?;
    Ok(())
}

async fn api_post(ctx: &TgContext, method: &str, payload: &Value) -> Result<Value> {
    let url = api_url(&ctx.cfg.telegram.bot_token, method);
    let resp = ctx
        .client
        .post(url)
        .timeout(Duration::from_secs(
            ctx.cfg.telegram.request_timeout_secs.max(3),
        ))
        .json(payload)
        .send()
        .await
        .with_context(|| format!("请求 Telegram {} 失败", method))?;
    let value: Value = resp
        .json()
        .await
        .with_context(|| format!("解析 Telegram {} 响应失败", method))?;
    if value.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(anyhow!("Telegram {} 返回异常: {}", method, value));
    }
    Ok(value)
}

fn api_url(token: &str, method: &str) -> String {
    format!("https://api.telegram.org/bot{}/{}", token, method)
}

fn split_message(text: &str, max_chars: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    let mut start = 0usize;
    while start < chars.len() {
        let end = (start + max_chars).min(chars.len());
        out.push(chars[start..end].iter().collect());
        start = end;
    }
    out
}

fn parse_schedule_input(text: &str) -> Result<Vec<String>> {
    let list = text
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    let normalized = normalized_schedule_list(&list).ok_or_else(|| anyhow!("未解析出有效时间"))?;
    Ok(serde_json::from_str(&normalized)?)
}

fn normalized_schedule_vec(list: &[String]) -> Vec<String> {
    normalized_schedule_list(list)
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default()
}

fn normalized_schedule_list(list: &[String]) -> Option<String> {
    let mut out = list
        .iter()
        .filter_map(|item| {
            let (hh, mm) = parse_single_time(item)?;
            Some(format!("{:02}:{:02}", hh, mm))
        })
        .collect::<Vec<_>>();
    out.sort();
    out.dedup();
    serde_json::to_string(&out).ok()
}

fn default_schedule_times_json() -> String {
    serde_json::to_string(&vec!["09:00".to_string(), "21:30".to_string()])
        .unwrap_or_else(|_| "[]".into())
}

fn parse_single_time(text: &str) -> Option<(u32, u32)> {
    let (hh, mm) = text.trim().split_once(':')?;
    let hh: u32 = hh.parse().ok()?;
    let mm: u32 = mm.parse().ok()?;
    if hh < 24 && mm < 60 {
        Some((hh, mm))
    } else {
        None
    }
}

/// HTML 转义：所有用户输入嵌入 HTML 文案前必须走这个，避免 `<`/`>`/`&`
/// 让 Telegram 解析失败或被错误渲染。Telegram HTML 模式只识别少量标签
/// （b/i/u/s/code/pre/a 等），其它都按字面量处理，所以三字符就够。
fn h(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

/// 20 格文本进度条：每格 5%，整格 █，半格（cell 余量 ≥ 0.5）▓，空格 ░。
fn progress_bar(pct: f64) -> String {
    let pct = pct.clamp(0.0, 100.0);
    const CELLS: usize = 20;
    let exact = pct * CELLS as f64 / 100.0;
    let filled = exact.floor() as usize;
    let remainder = exact - filled as f64;
    let mut s = String::with_capacity(CELLS * 4 + 2);
    s.push('[');
    for i in 0..CELLS {
        if i < filled {
            s.push('█');
        } else if i == filled && remainder >= 0.5 {
            s.push('▓');
        } else {
            s.push('░');
        }
    }
    s.push(']');
    s
}

fn status_emoji(enabled: bool) -> &'static str {
    if enabled {
        "✅"
    } else {
        "⛔"
    }
}

fn quota_alert_emoji(level: u8) -> &'static str {
    match level {
        100 => "🚨",
        90 => "⚠️",
        80 => "🔔",
        _ => "📊",
    }
}

/// 解析 telegram.timezone 配置。支持：
/// - 空串 / "UTC" / "Z" → +00:00
/// - **不走 DST 的** IANA 别名（Asia/Shanghai、Asia/Tokyo、Asia/Dubai、
///   Australia/Brisbane 等）→ 写死的固定偏移
/// - "+HH:MM" / "-HH:MM" / "+HHMM" / "+HH"
///
/// 不引入 chrono-tz 是为了不增加二进制体积。会 DST 的时区
/// （Europe/London、America/*、Australia/Sydney 等）**故意不在别名表里**——
/// 给它们一个固定偏移会在夏令时期间整整偏 1 小时，比 fallback 到默认更危险。
/// 这些时区请用显式偏移（如 `-05:00`）；用户填 `Europe/London` 等会回落到
/// 默认偏移并打 warn 提示。
fn parse_timezone(s: &str) -> Option<FixedOffset> {
    let s = s.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("UTC") || s.eq_ignore_ascii_case("Z") {
        return FixedOffset::east_opt(0);
    }
    // 仅保留全年无 DST 的 IANA 别名。其他时区一律走显式 ±HH:MM 偏移。
    let aliased_secs = match s {
        "Asia/Shanghai" | "Asia/Hong_Kong" | "Asia/Taipei" | "Asia/Singapore" | "Asia/Macau"
        | "Asia/Kuala_Lumpur" | "Asia/Manila" => Some(8 * 3600),
        "Asia/Tokyo" | "Asia/Seoul" => Some(9 * 3600),
        "Asia/Bangkok" | "Asia/Ho_Chi_Minh" | "Asia/Jakarta" => Some(7 * 3600),
        "Asia/Kolkata" | "Asia/Calcutta" => Some(5 * 3600 + 30 * 60),
        "Asia/Dubai" => Some(4 * 3600),
        "Australia/Brisbane" | "Australia/Perth" => Some(10 * 3600),
        _ => None,
    };
    if let Some(secs) = aliased_secs {
        return FixedOffset::east_opt(secs);
    }
    let (sign, rest) = match s.chars().next()? {
        '+' => (1i32, &s[1..]),
        '-' => (-1i32, &s[1..]),
        _ => return None,
    };
    let (hh, mm) = if let Some((h, m)) = rest.split_once(':') {
        (h, m)
    } else if rest.len() == 4 {
        (&rest[..2], &rest[2..])
    } else if rest.len() == 2 || rest.len() == 1 {
        (rest, "0")
    } else {
        return None;
    };
    let h: i32 = hh.parse().ok()?;
    let m: i32 = mm.parse().ok()?;
    if !(0..=14).contains(&h) || !(0..=59).contains(&m) {
        return None;
    }
    FixedOffset::east_opt(sign * (h * 3600 + m * 60))
}

#[cfg(test)]
mod tests {
    use super::{h, normalized_schedule_vec, parse_schedule_input, parse_timezone, progress_bar};
    use chrono::FixedOffset;

    #[test]
    fn parse_schedule_accepts_multiple_times() {
        let times = parse_schedule_input("21:30, 09:00,21:30").unwrap();
        assert_eq!(times, vec!["09:00", "21:30"]);
    }

    #[test]
    fn normalized_schedule_drops_invalid_times() {
        assert_eq!(
            normalized_schedule_vec(&["09:00".into(), "25:00".into(), "21:30".into()]),
            vec!["09:00", "21:30"]
        );
    }

    #[test]
    fn parse_timezone_iana_aliases() {
        assert_eq!(
            parse_timezone("Asia/Shanghai"),
            FixedOffset::east_opt(8 * 3600)
        );
        assert_eq!(
            parse_timezone("Asia/Tokyo"),
            FixedOffset::east_opt(9 * 3600)
        );
        assert_eq!(
            parse_timezone("Australia/Brisbane"),
            FixedOffset::east_opt(10 * 3600)
        );
    }

    #[test]
    fn parse_timezone_dst_aliases_rejected() {
        // 这些时区有夏令时，给它们一个固定偏移会在 DST 期间整整偏 1 小时，
        // 比 fallback 到默认 +08:00 + warn 更危险 —— 故意不在别名表里。
        // 用户应该用显式偏移（如 -05:00）。
        assert_eq!(parse_timezone("Europe/London"), None);
        assert_eq!(parse_timezone("Europe/Paris"), None);
        assert_eq!(parse_timezone("America/New_York"), None);
        assert_eq!(parse_timezone("America/Los_Angeles"), None);
        assert_eq!(parse_timezone("Australia/Sydney"), None);
    }

    #[test]
    fn parse_timezone_offset_forms() {
        assert_eq!(
            parse_timezone("+05:30"),
            FixedOffset::east_opt(5 * 3600 + 30 * 60)
        );
        assert_eq!(parse_timezone("-08:00"), FixedOffset::east_opt(-8 * 3600));
        assert_eq!(parse_timezone("+0800"), FixedOffset::east_opt(8 * 3600));
        assert_eq!(parse_timezone("+08"), FixedOffset::east_opt(8 * 3600));
    }

    #[test]
    fn parse_timezone_special_values() {
        assert_eq!(parse_timezone(""), FixedOffset::east_opt(0));
        assert_eq!(parse_timezone("UTC"), FixedOffset::east_opt(0));
        assert_eq!(parse_timezone("z"), FixedOffset::east_opt(0));
    }

    #[test]
    fn parse_timezone_rejects_invalid() {
        assert_eq!(parse_timezone("invalid"), None);
        assert_eq!(parse_timezone("+25:00"), None);
        assert_eq!(parse_timezone("+abc"), None);
    }

    #[test]
    fn progress_bar_boundaries() {
        assert_eq!(progress_bar(0.0), "[░░░░░░░░░░░░░░░░░░░░]");
        assert_eq!(progress_bar(50.0), "[██████████░░░░░░░░░░]");
        assert_eq!(progress_bar(100.0), "[████████████████████]");
    }

    #[test]
    fn progress_bar_half_cell_when_remainder_at_least_half() {
        // 23.4% × 20 = 4.68 → 4 整 + 0.68 余 (≥0.5) → ▓ + 15 ░
        assert_eq!(progress_bar(23.4), "[████▓░░░░░░░░░░░░░░░]");
        // 22% × 20 = 4.4 → 4 整 + 0.4 余 (<0.5) → ░
        assert_eq!(progress_bar(22.0), "[████░░░░░░░░░░░░░░░░]");
    }

    #[test]
    fn progress_bar_clamps_overflow() {
        assert_eq!(progress_bar(150.0), "[████████████████████]");
        assert_eq!(progress_bar(-5.0), "[░░░░░░░░░░░░░░░░░░░░]");
    }

    #[test]
    fn html_escape_replaces_dangerous_chars() {
        assert_eq!(h("a<b>&c"), "a&lt;b&gt;&amp;c");
        // 单/双引号在 Telegram HTML 文本中是安全的，不必 escape
        assert_eq!(h("\"'"), "\"'");
        // 用户名里夹 HTML 标签会被打散成字面量
        assert_eq!(h("<script>x</script>"), "&lt;script&gt;x&lt;/script&gt;");
    }
}
