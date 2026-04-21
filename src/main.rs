mod cli;
mod core;
mod db;
mod model;
mod service;
mod tui;

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use tokio::sync::mpsc;
use tracing_subscriber::EnvFilter;

use cli::{Cli, Commands};
use model::config::AppConfig;

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<()> {
    init_tracing();
    let cli = Cli::parse();
    let cfg_path = resolve_config_path(cli.config.as_deref());
    let cfg = load_or_init_config(&cfg_path, matches!(cli.command, Some(Commands::Daemon) | Some(Commands::Tui) | None))?;

    if let Some(parent) = Path::new(&cfg.db.path).parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("创建数据目录 {} 失败", parent.display()))?;
    }
    let pool = db::init_pool(&cfg.db.path).await?;

    match cli.command.unwrap_or(Commands::Tui) {
        Commands::Users          => run_user(cli::user::UserCommands::List, &pool, &cfg).await,
        Commands::Add(a)         => run_user(cli::user::UserCommands::Add { name: a.name, quota: a.quota, reset_day: a.reset_day, expire: a.expire }, &pool, &cfg).await,
        Commands::Del { name }   => run_user(cli::user::UserCommands::Del { name }, &pool, &cfg).await,
        Commands::On { name }    => set_user_enabled(&pool, &name, true, &cfg).await,
        Commands::Off { name }   => set_user_enabled(&pool, &name, false, &cfg).await,
        Commands::Reset { name } => run_user(cli::user::UserCommands::Reset { name }, &pool, &cfg).await,
        Commands::Info { name }  => run_user(cli::user::UserCommands::Info { name }, &pool, &cfg).await,
        Commands::Sub { name }   => run_user(cli::user::UserCommands::Sub { name }, &pool, &cfg).await,
        Commands::Pkg(a)         => run_user(cli::user::UserCommands::Package { name: a.name, quota: a.quota, reset_day: a.reset_day, expire: a.expire }, &pool, &cfg).await,
        Commands::Grant { name, tag }    => run_user(cli::user::UserCommands::Grant { name, tag }, &pool, &cfg).await,
        Commands::Revoke { name, tag }   => run_user(cli::user::UserCommands::Revoke { name, tag }, &pool, &cfg).await,
        Commands::GrantAll { name }      => run_user(cli::user::UserCommands::GrantAll { name }, &pool, &cfg).await,
        Commands::Allowed { name }       => run_user(cli::user::UserCommands::Allowed { name }, &pool, &cfg).await,
        Commands::Nodes          => run_node(cli::node::NodeCommands::List, &cfg).await,
        Commands::AddNode(a)     => run_node(cli::node::NodeCommands::Add(a), &cfg).await,
        Commands::Export { name } => run_node(cli::node::NodeCommands::Export { name }, &cfg).await,
        Commands::Check          => run_check(&cfg),
        Commands::Start          => run_start(&cfg),
        Commands::Stop           => run_stop(&cfg),
        Commands::Reload         => run_reload(&cfg),
        Commands::Status         => run_status(&cfg).await,
        Commands::User(a)        => run_user(a.command, &pool, &cfg).await,
        Commands::Node(a)        => run_node(a.command, &cfg).await,
        Commands::Kernel(a)      => run_kernel(a.command, &cfg).await,
        Commands::Token(a)       => run_token(a.command, &pool, &cfg).await,
        Commands::Nginx(a)       => run_nginx(a.command, &cfg),
        Commands::Daemon         => run_daemon(pool, cfg).await,
        Commands::Tui            => run_tui(pool, cfg).await,
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

/// 按优先级解析配置路径：CLI --config > $SB_MANAGER_CONFIG > /etc > ./
fn resolve_config_path(arg: Option<&str>) -> PathBuf {
    if let Some(p) = arg { return PathBuf::from(p); }
    if let Ok(p) = std::env::var("SB_MANAGER_CONFIG") { return PathBuf::from(p); }
    let etc = PathBuf::from("/etc/sing-box-manager/config.toml");
    if etc.exists() { return etc; }
    PathBuf::from("config.toml")
}

/// 读取配置；仅在长运行命令（tui/daemon/default）下允许自动创建。
fn load_or_init_config(path: &Path, allow_create: bool) -> Result<AppConfig> {
    if path.exists() {
        let s = std::fs::read_to_string(path)
            .with_context(|| format!("读取配置 {} 失败", path.display()))?;
        Ok(toml::from_str(&s).with_context(|| format!("解析配置 {} 失败", path.display()))?)
    } else if allow_create {
        let d = AppConfig::default();
        if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).ok(); }
        std::fs::write(path, toml::to_string_pretty(&d)?)
            .with_context(|| format!("写入默认配置 {} 失败", path.display()))?;
        Ok(d)
    } else {
        anyhow::bail!("配置文件不存在: {} （使用 --config 指定或运行 tui/daemon 自动生成）", path.display())
    }
}

async fn run_daemon(pool: sqlx::SqlitePool, cfg: AppConfig) -> Result<()> {
    use service::traffic_service::{self, TrafficEvent};

    // 给老库补订阅 token
    if let Ok(n) = service::user_service::ensure_sub_tokens(&pool).await {
        if n > 0 { tracing::info!(filled = n, "为历史用户补发订阅 token"); }
    }

    // 订阅 HTTP 服务
    if cfg.subscription.enabled {
        let pool_sub = pool.clone();
        let cfg_sub = std::sync::Arc::new(cfg.clone());
        tokio::spawn(async move {
            if let Err(e) = service::sub_server::run(pool_sub, cfg_sub).await {
                tracing::error!("订阅服务错误: {}", e);
            }
        });
    }

    let (tx, mut rx) = mpsc::channel::<TrafficEvent>(128);

    // 事件汇聚 -> 日志
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            match event {
                TrafficEvent::Synced(d) => {
                    let up: i64 = d.iter().map(|x| x.delta_up).sum();
                    let dn: i64 = d.iter().map(|x| x.delta_down).sum();
                    tracing::info!(users = d.len(), up_bytes = up, down_bytes = dn, "流量同步完成");
                }
                TrafficEvent::QuotaAlert(n, p) => tracing::warn!(user = %n, used_percent = p, "达到流量阈值"),
                TrafficEvent::AutoControl(c) => for item in c { tracing::info!(event = %item, "自动控制"); },
                TrafficEvent::GrpcConnected => tracing::info!("gRPC 已连接"),
                TrafficEvent::GrpcError(e) => tracing::warn!(error = %e, "gRPC 同步失败"),
                TrafficEvent::Tick => {}
            }
        }
    });

    // 带指数退避的重连循环
    let mut backoff_secs = 1u64;
    loop {
        match core::grpc::connect(&cfg.singbox.grpc_addr).await {
            Ok(client) => {
                backoff_secs = 1;
                traffic_service::run_until_disconnect(
                    pool.clone(), client,
                    cfg.stats.sync_interval_secs,
                    cfg.stats.quota_alert_percent,
                    tx.clone(),
                ).await;
                tracing::warn!("流量同步任务退出，准备重连");
            }
            Err(e) => {
                tracing::warn!(addr = %cfg.singbox.grpc_addr, error = %e, "连接 gRPC 失败");
                if let Err(err) = service::user_service::apply_automatic_controls(&pool).await {
                    tracing::error!(error = %err, "执行自动控制失败");
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
        backoff_secs = (backoff_secs * 2).min(60);
    }
}

async fn run_tui(pool: sqlx::SqlitePool, cfg: AppConfig) -> Result<()> {
    use service::traffic_service::{self, TrafficEvent};

    // 给老库补订阅 token
    let _ = service::user_service::ensure_sub_tokens(&pool).await;

    // 订阅 HTTP 服务（TUI 模式也开，方便开发测试）
    if cfg.subscription.enabled {
        let pool_sub = pool.clone();
        let cfg_sub = std::sync::Arc::new(cfg.clone());
        tokio::spawn(async move {
            if let Err(e) = service::sub_server::run(pool_sub, cfg_sub).await {
                tracing::warn!("订阅服务退出: {}", e);
            }
        });
    }

    let users = service::user_service::list_users(&pool).await.unwrap_or_default();
    let nodes = if Path::new(&cfg.singbox.config_path).exists() {
        core::config::load(&cfg.singbox.config_path)
            .map(|c| service::node_service::list_nodes(&c))
            .unwrap_or_default()
    } else { vec![] };

    let proc = core::singbox::SingboxProcess::new(&cfg.singbox.binary_path, &cfg.singbox.config_path);
    let mut app = tui::app::AppState::new();
    app.singbox_running = proc.is_running();

    let (tx, rx) = mpsc::channel::<TrafficEvent>(128);

    // 后台流量/重连任务；失败时自动重试（与 daemon 相同）
    {
        let pool_bg = pool.clone();
        let cfg_bg  = cfg.clone();
        let tx_bg   = tx.clone();
        tokio::spawn(async move {
            let mut backoff = 1u64;
            loop {
                match core::grpc::connect(&cfg_bg.singbox.grpc_addr).await {
                    Ok(client) => {
                        backoff = 1;
                        traffic_service::run_until_disconnect(
                            pool_bg.clone(), client,
                            cfg_bg.stats.sync_interval_secs,
                            cfg_bg.stats.quota_alert_percent,
                            tx_bg.clone(),
                        ).await;
                    }
                    Err(e) => { let _ = tx_bg.send(TrafficEvent::GrpcError(e.to_string())).await; }
                }
                tokio::time::sleep(Duration::from_secs(backoff)).await;
                backoff = (backoff * 2).min(60);
            }
        });
    }

    // UI Tick 节拍（1s，总是有）；不依赖 gRPC
    {
        let tx_tick = tx.clone();
        tokio::spawn(async move {
            let mut iv = tokio::time::interval(Duration::from_secs(1));
            iv.tick().await;
            loop {
                iv.tick().await;
                if tx_tick.send(TrafficEvent::Tick).await.is_err() { break; }
            }
        });
    }

    tui::runner::run(app, rx, users, nodes, pool, cfg).await
}

async fn run_user(cmd: cli::user::UserCommands, pool: &sqlx::SqlitePool, cfg: &AppConfig) -> Result<()> {
    use cli::user::UserCommands;
    use model::user::User;
    use service::user_service;
    match cmd {
        UserCommands::List => {
            let us = user_service::list_users(pool).await?;
            println!("{:<16}{:<8}{:<12}{:<12}{:<12}{:<12}{:<10}",
                "用户名", "状态", "上行", "下行", "总量", "配额", "到期");
            println!("{}", "─".repeat(82));
            for u in &us {
                println!("{:<16}{:<8}{:<12}{:<12}{:<12}{:<12}{:<10}",
                    u.name, if u.enabled {"启用"} else {"禁用"},
                    User::format_bytes(u.used_up_bytes),
                    User::format_bytes(u.used_down_bytes),
                    User::format_bytes(u.used_total_bytes()),
                    if u.quota_gb <= 0.0 {"不限".into()} else {format!("{}GB", u.quota_gb)},
                    if u.expire_at.is_empty() {"永久".into()} else {u.expire_at.clone()});
            }
        }
        UserCommands::Info { name } => {
            let u = db::user_repo::get(pool, &name).await?
                .ok_or_else(|| anyhow::anyhow!("用户不存在: {}", name))?;
            println!("用户名: {}\n状态:   {}\n上行:   {}\n下行:   {}\n总量:   {}\n配额:   {}\n已用%:  {:.1}%\n到期:   {}",
                u.name, if u.enabled {"启用"} else {"禁用"},
                User::format_bytes(u.used_up_bytes),
                User::format_bytes(u.used_down_bytes),
                User::format_bytes(u.used_total_bytes()),
                if u.quota_gb <= 0.0 {"不限".into()} else {format!("{} GB", u.quota_gb)},
                u.quota_used_percent(),
                if u.expire_at.is_empty() {"永久"} else {&u.expire_at});
            if u.allow_all_nodes {
                println!("节点:   全部");
            } else {
                let t = u.allowed_tags();
                if t.is_empty() { println!("节点:   无"); }
                else { println!("节点:   {}", t.join(", ")); }
            }
            print_sub_url(&u, &cfg.subscription.public_base);
        }
        UserCommands::Add { name, quota, reset_day, expire } => {
            let u = user_service::add_user(pool, &name, quota, reset_day, &expire).await?;
            apply_runtime_changes(pool, cfg).await?;
            println!("✓ 用户 '{}' 已添加", name);
            print_sub_url(&u, &cfg.subscription.public_base);
        }
        UserCommands::Del { name } => {
            user_service::delete_user(pool, &name).await?;
            apply_runtime_changes(pool, cfg).await?;
            println!("✓ 已删除 '{}'", name);
        }
        UserCommands::Toggle { name } => {
            let s = user_service::toggle_user(pool, &name).await?;
            apply_runtime_changes(pool, cfg).await?;
            println!("✓ '{}' 已{}", name, if s {"启用"} else {"禁用"});
        }
        UserCommands::Reset { name } => {
            user_service::reset_traffic(pool, &name).await?;
            apply_runtime_changes(pool, cfg).await?;
            println!("✓ '{}' 流量已重置", name);
        }
        UserCommands::Package { name, quota, reset_day, expire } => {
            user_service::update_package(pool, &name, quota, reset_day, expire.as_deref()).await?;
            apply_runtime_changes(pool, cfg).await?;
            println!("✓ '{}' 套餐已更新", name);
        }
        UserCommands::Grant { name, tag } => {
            user_service::grant_node(pool, &name, &tag).await?;
            apply_runtime_changes(pool, cfg).await?;
            println!("✓ '{}' 已获得节点 '{}' 的访问", name, tag);
        }
        UserCommands::Revoke { name, tag } => {
            let existing: Vec<String> = if Path::new(&cfg.singbox.config_path).exists() {
                core::config::load(&cfg.singbox.config_path)
                    .map(|v| core::config::list_tags(&v)).unwrap_or_default()
            } else { vec![] };
            user_service::revoke_node(pool, &name, &tag, &existing).await?;
            apply_runtime_changes(pool, cfg).await?;
            println!("✓ '{}' 已撤销节点 '{}' 的访问", name, tag);
        }
        UserCommands::GrantAll { name } => {
            user_service::grant_all_nodes(pool, &name).await?;
            apply_runtime_changes(pool, cfg).await?;
            println!("✓ '{}' 已恢复全部节点可用", name);
        }
        UserCommands::Allowed { name } => {
            let u = db::user_repo::get(pool, &name).await?
                .ok_or_else(|| anyhow::anyhow!("用户不存在: {}", name))?;
            if u.allow_all_nodes {
                println!("{} 当前全部节点可用", name);
            } else {
                let tags = u.allowed_tags();
                if tags.is_empty() { println!("{} 无可用节点", name); }
                else {
                    println!("{} 可用节点:", name);
                    for t in &tags { println!("  - {}", t); }
                }
            }
        }
        UserCommands::Sub { name } => {
            if !Path::new(&cfg.singbox.config_path).exists() {
                println!("config.json 不存在"); return Ok(());
            }
            let config = core::config::load(&cfg.singbox.config_path)?;
            let ip = service::node_service::get_server_ip().await;
            let links = service::sub_service::generate_links(&config, &name, &ip)?;
            if links.is_empty() { println!("用户 '{}' 无可用节点", name); }
            else { for l in &links { println!("[{}] {}", l.protocol, l.link); } }
        }
    }
    Ok(())
}

async fn run_node(cmd: cli::node::NodeCommands, cfg: &AppConfig) -> Result<()> {
    use cli::node::NodeCommands;
    if !Path::new(&cfg.singbox.config_path).exists() {
        if matches!(cmd, NodeCommands::Add(_)) {
            let empty = serde_json::json!({ "inbounds": [], "outbounds": [] });
            if let Some(parent) = Path::new(&cfg.singbox.config_path).parent() {
                std::fs::create_dir_all(parent).ok();
            }
            core::config::save(&cfg.singbox.config_path, &empty)?;
        } else {
            println!("config.json 不存在");
            return Ok(());
        }
    }
    let mut config = core::config::load(&cfg.singbox.config_path)?;
    match cmd {
        NodeCommands::List => {
            let ns = service::node_service::list_nodes(&config);
            println!("{:<22}{:<16}{:<8}{:<8}", "Tag", "协议", "端口", "用户数");
            println!("{}", "─".repeat(56));
            for n in &ns {
                println!("{:<22}{:<16}{:<8}{:<8}", n.tag, n.protocol, n.listen_port, n.user_count);
            }
        }
        NodeCommands::Export { name } => {
            let ip = service::node_service::get_server_ip().await;
            let ls = service::sub_service::generate_links(&config, &name, &ip)?;
            println!("# 订阅 (Base64)\n{}", service::sub_service::generate_subscription(&ls));
            println!("\n# 明文");
            for l in &ls { println!("{}", l.link); }
        }
        NodeCommands::Add(args) => {
            let protocol = model::node::Protocol::try_from(args.protocol.as_str())?;
            let req = model::node::AddNodeRequest {
                tag: args.tag, protocol,
                listen_port: args.port,
                server_name: args.server_name,
                path: args.path,
                port_reuse: args.port_reuse,
            };
            let meta = core::config::add_node(&mut config, &req)?;
            core::config::save(&cfg.singbox.config_path, &config)?;
            let proc = core::singbox::SingboxProcess::new(&cfg.singbox.binary_path, &cfg.singbox.config_path);
            proc.check_config()?;
            if matches!(proc.is_running(), Some(true)) { proc.reload()?; }
            match meta {
                core::config::AddNodeMeta::RealityKeys { public_key, short_id } => {
                    println!("✓ 节点已添加");
                    println!("  reality public_key: {}", public_key);
                    println!("  reality short_id:   {}", short_id);
                    println!("  （已写入 config；客户端订阅链接会自动带上）");
                }
                core::config::AddNodeMeta::Plain => println!("✓ 节点已添加"),
            }
        }
    }
    Ok(())
}

pub async fn apply_runtime_changes(pool: &sqlx::SqlitePool, cfg: &AppConfig) -> Result<()> {
    if !Path::new(&cfg.singbox.config_path).exists() { return Ok(()); }
    let mut config = core::config::load(&cfg.singbox.config_path)?;
    let users = service::user_service::list_users(pool).await?;
    core::config::sync_users(&mut config, &users, &cfg.singbox.grpc_addr);
    core::config::save(&cfg.singbox.config_path, &config)?;
    let proc = core::singbox::SingboxProcess::new(&cfg.singbox.binary_path, &cfg.singbox.config_path);
    proc.check_config()?;
    if matches!(proc.is_running(), Some(true)) { proc.reload()?; }
    Ok(())
}

async fn set_user_enabled(pool: &sqlx::SqlitePool, name: &str, enabled: bool, cfg: &AppConfig) -> Result<()> {
    let user = db::user_repo::get(pool, name).await?
        .ok_or_else(|| anyhow::anyhow!("用户不存在: {}", name))?;
    if user.enabled != enabled {
        db::user_repo::set_enabled(pool, name, enabled).await?;
        apply_runtime_changes(pool, cfg).await?;
    }
    println!("✓ '{}' 已{}", name, if enabled { "启用" } else { "禁用" });
    Ok(())
}

fn run_check(cfg: &AppConfig) -> Result<()> {
    let proc = core::singbox::SingboxProcess::new(&cfg.singbox.binary_path, &cfg.singbox.config_path);
    proc.check_config()?;
    println!("✓ sing-box 配置校验通过");
    Ok(())
}

fn run_start(cfg: &AppConfig) -> Result<()> {
    core::singbox::SingboxProcess::new(&cfg.singbox.binary_path, &cfg.singbox.config_path).start()?;
    println!("✓ sing-box 已启动");
    Ok(())
}

fn run_stop(cfg: &AppConfig) -> Result<()> {
    core::singbox::SingboxProcess::new(&cfg.singbox.binary_path, &cfg.singbox.config_path).stop()?;
    println!("✓ sing-box 已停止");
    Ok(())
}

fn run_reload(cfg: &AppConfig) -> Result<()> {
    let proc = core::singbox::SingboxProcess::new(&cfg.singbox.binary_path, &cfg.singbox.config_path);
    proc.check_config()?;
    proc.reload()?;
    println!("✓ sing-box 已重载");
    Ok(())
}

async fn run_kernel(cmd: cli::kernel::KernelCommands, cfg: &AppConfig) -> Result<()> {
    use cli::kernel::KernelCommands as K;
    match cmd {
        K::Status => {
            let s = core::singbox::status();
            println!("安装:   {}", if s.installed {"是"} else {"否"});
            println!("路径:   {}", s.binary_path.as_deref().unwrap_or("—"));
            println!("版本:   {}", s.version.as_deref().unwrap_or("—"));
            println!("运行:   {}", match s.running {
                Some(true) => "运行中", Some(false) => "未运行", None => "未知",
            });
            println!("自启:   {}", if s.enabled {"已启用"} else {"未启用"});
        }
        K::Install          => { core::singbox::install_latest()?; println!("✓ 官方版 sing-box 已安装"); }
        K::InstallV2rayApi  => {
            core::singbox::install_v2rayapi(&cfg.kernel.update_repo).await?;
            println!("✓ v2ray_api 版 sing-box 已安装");
        }
        K::Uninstall => { core::singbox::uninstall()?; println!("✓ sing-box 已卸载"); }
        K::Start     => { core::singbox::start()?;     println!("✓ sing-box 已启动"); }
        K::Stop      => { core::singbox::stop()?;      println!("✓ sing-box 已停止"); }
        K::Restart   => { core::singbox::restart()?;   println!("✓ sing-box 已重启"); }
        K::Enable    => { core::singbox::enable()?;    println!("✓ sing-box 已设为开机自启"); }
        K::Disable   => { core::singbox::disable()?;   println!("✓ sing-box 已关闭开机自启"); }
    }
    Ok(())
}

async fn run_status(cfg: &AppConfig) -> Result<()> {
    let proc = core::singbox::SingboxProcess::new(&cfg.singbox.binary_path, &cfg.singbox.config_path);
    let running = match proc.is_running() {
        Some(true) => "运行中",
        Some(false) => "未运行",
        None => "未知",
    };
    let grpc = if core::grpc::connect(&cfg.singbox.grpc_addr).await.is_ok() { "已连接" } else { "未连接" };
    println!("sing-box: {}", running);
    println!("gRPC:     {} ({})", grpc, cfg.singbox.grpc_addr);
    println!("config:   {}", cfg.singbox.config_path);
    println!("db:       {}", cfg.db.path);
    Ok(())
}

async fn run_token(cmd: cli::token::TokenCommands, pool: &sqlx::SqlitePool, cfg: &AppConfig) -> Result<()> {
    use cli::token::TokenCommands as T;
    match cmd {
        T::Show { name } => {
            let u = db::user_repo::get(pool, &name).await?
                .ok_or_else(|| anyhow::anyhow!("用户不存在: {}", name))?;
            print_sub_url(&u, &cfg.subscription.public_base);
        }
        T::Regen { name } => {
            let t = service::user_service::regen_sub_token(pool, &name).await?;
            println!("✓ '{}' 的 token 已重置", name);
            let u = db::user_repo::get(pool, &name).await?
                .ok_or_else(|| anyhow::anyhow!("用户不存在: {}", name))?;
            print_sub_url(&u, &cfg.subscription.public_base);
            drop(t);
        }
        T::Revoke { name } => {
            service::user_service::revoke_sub_token(pool, &name).await?;
            println!("✓ '{}' 的订阅已关闭；`sb token regen {}` 可恢复", name, name);
        }
    }
    Ok(())
}

fn print_sub_url(u: &model::user::User, public_base: &str) {
    if u.sub_token.is_empty() {
        println!("(该用户无 token，运行 sb token regen {} 生成)", u.name);
        return;
    }
    if public_base.is_empty() {
        println!("Token: {}", u.sub_token);
        println!("(未设置 [subscription].public_base，无法拼完整 URL)");
    } else {
        let base = public_base.trim_end_matches('/');
        println!("sing-box: {}/sub/{}", base, u.sub_token);
        println!("mihomo:   {}/sub/{}?type=clash", base, u.sub_token);
    }
}

fn run_nginx(cmd: cli::nginx::NginxCommands, cfg: &AppConfig) -> Result<()> {
    use cli::nginx::NginxCommands as N;
    match cmd {
        N::Status => {
            let s = core::nginx::status(&cfg.subscription.nginx_conf);
            println!("安装:       {}", if s.installed {"是"} else {"否"});
            println!("路径:       {}", s.binary_path.as_deref().unwrap_or("—"));
            println!("版本:       {}", s.version.as_deref().unwrap_or("—"));
            println!("运行:       {}", match s.running {
                Some(true) => "运行中", Some(false) => "未运行", None => "未知",
            });
            println!("自启:       {}", if s.enabled {"已启用"} else {"未启用"});
            println!("sb-manager 配置: {} ({})", cfg.subscription.nginx_conf,
                if s.conf_exists {"已生成"} else {"未生成"});
        }
        N::Install => { core::nginx::install_via_pkg()?; println!("✓ nginx 已安装"); }
        N::Start   => { core::nginx::start()?;   println!("✓ nginx 已启动"); }
        N::Stop    => { core::nginx::stop()?;    println!("✓ nginx 已停止"); }
        N::Restart => { core::nginx::restart()?; println!("✓ nginx 已重启"); }
        N::Reload  => { core::nginx::reload()?;  println!("✓ nginx 已重载"); }
        N::Enable  => { core::nginx::enable()?;  println!("✓ nginx 已开机自启"); }
        N::Disable => { core::nginx::disable()?; println!("✓ nginx 已关闭自启"); }
        N::Test    => { let out = core::nginx::test_config()?; println!("{}", out); }
        N::GenConf => {
            core::nginx::generate_conf(
                &cfg.subscription.nginx_conf,
                &cfg.subscription.public_base,
                &cfg.subscription.listen,
            )?;
            println!("✓ 已写入 {}", cfg.subscription.nginx_conf);
            println!("请编辑该文件里的 ssl_certificate / ssl_certificate_key 路径，再 sb nginx test && sb nginx reload");
        }
    }
    Ok(())
}
