use anyhow::Result;
use crossterm::{
    event::{self, Event as CE, KeyCode, KeyEvent, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, layout::{Constraint, Direction, Layout}, Terminal};
use std::{io, sync::Arc, time::Duration};
use tokio::sync::mpsc;

use crate::{
    model::{config::AppConfig, node::InboundNode, user::User},
    service::traffic_service::TrafficEvent,
    tui::{app::{AppState, Page, StatusLevel}, pages, widgets},
};

/// UI 线程以外产生的异步结果，回传给主循环刷新状态。
#[derive(Debug)]
pub enum UiEvent {
    UsersRefreshed(Vec<User>),
    NodesRefreshed(Vec<InboundNode>),
    SingboxRunning(Option<bool>),
    UserEnabled { name: String, enabled: bool },
    TrafficReset { name: String },
    SubscriptionExported { name: String, text: String },
    Status { msg: String, level: StatusLevel },
}

pub async fn run(
    mut s: AppState,
    mut rx: mpsc::Receiver<TrafficEvent>,
    users: Vec<User>,
    nodes: Vec<InboundNode>,
    pool: sqlx::SqlitePool,
    cfg: AppConfig,
) -> Result<()> {
    s.users = users;
    s.nodes = nodes;
    enable_raw_mode()?;
    let mut out = io::stdout();
    execute!(out, EnterAlternateScreen)?;
    let mut term = Terminal::new(CrosstermBackend::new(out))?;
    let r = event_loop(&mut term, &mut s, &mut rx, Arc::new(pool), Arc::new(cfg)).await;
    disable_raw_mode()?;
    execute!(term.backend_mut(), LeaveAlternateScreen)?;
    term.show_cursor()?;
    r
}

async fn event_loop(
    term: &mut Terminal<CrosstermBackend<io::Stdout>>,
    s: &mut AppState,
    rx: &mut mpsc::Receiver<TrafficEvent>,
    pool: Arc<sqlx::SqlitePool>,
    cfg: Arc<AppConfig>,
) -> Result<()> {
    let (ui_tx, mut ui_rx) = mpsc::channel::<UiEvent>(64);

    loop {
        term.draw(|f| {
            let area = f.area();
            let c = Layout::default().direction(Direction::Vertical)
                .constraints([Constraint::Length(2), Constraint::Min(0), Constraint::Length(1)])
                .split(area);
            widgets::tab_bar::render(f, c[0], s);
            match s.page {
                Page::Dashboard => pages::dashboard::render(f, c[1], s),
                Page::Users     => pages::users::render(f, c[1], s),
                Page::Nodes     => pages::nodes::render(f, c[1], s),
                Page::Logs      => pages::logs::render(f, c[1], s),
            }
            widgets::status_bar::render(f, c[2], s);
        })?;

        tokio::select! {
            biased;
            _ = tokio::time::sleep(Duration::from_millis(50)) => {
                if event::poll(Duration::from_millis(0))? {
                    if let CE::Key(k) = event::read()? {
                        if k.kind == KeyEventKind::Press {
                            if is_quit(&k) { return Ok(()); }
                            handle_key(s, k, pool.clone(), cfg.clone(), ui_tx.clone());
                        }
                    }
                }
            }
            Some(ev) = ui_rx.recv() => apply_ui_event(s, ev),
            Some(bg) = rx.recv() => apply_traffic_event(s, bg),
        }
    }
}

fn is_quit(k: &KeyEvent) -> bool {
    use crossterm::event::KeyModifiers;
    matches!(k.code, KeyCode::Char('q') | KeyCode::Char('Q'))
        || (matches!(k.code, KeyCode::Char('c')) && k.modifiers == KeyModifiers::CONTROL)
}

fn apply_ui_event(s: &mut AppState, ev: UiEvent) {
    match ev {
        UiEvent::UsersRefreshed(u) => s.users = u,
        UiEvent::NodesRefreshed(n) => s.nodes = n,
        UiEvent::SingboxRunning(v) => s.singbox_running = v,
        UiEvent::UserEnabled { name, enabled } => {
            if let Some(u) = s.users.iter_mut().find(|u| u.name == name) { u.enabled = enabled; }
            s.push_log(format!("[INFO] {} 已{}", name, if enabled { "启用" } else { "禁用" }));
        }
        UiEvent::TrafficReset { name } => {
            if let Some(u) = s.users.iter_mut().find(|u| u.name == name) {
                u.used_up_bytes = 0; u.used_down_bytes = 0; u.manual_bytes = 0;
            }
            s.push_log(format!("[INFO] {} 流量已重置", name));
        }
        UiEvent::SubscriptionExported { name, text } => {
            s.last_subscription = Some(text);
            s.push_log(format!("[INFO] 已导出 {} 的订阅到日志页", name));
            s.set_status(format!("{} 的订阅已输出到日志页", name), StatusLevel::Warn);
            s.page = Page::Logs;
        }
        UiEvent::Status { msg, level } => s.set_status(msg, level),
    }
}

fn apply_traffic_event(s: &mut AppState, ev: TrafficEvent) {
    match ev {
        TrafficEvent::Tick => s.uptime_secs += 1,
        TrafficEvent::Synced(deltas) => {
            for d in &deltas {
                if let Some(u) = s.users.iter_mut().find(|u| u.name == d.username) {
                    u.used_up_bytes += d.delta_up;
                    u.used_down_bytes += d.delta_down;
                    u.last_live_up = d.new_live_up;
                    u.last_live_down = d.new_live_down;
                }
            }
            let tu: i64 = deltas.iter().map(|d| d.delta_up).sum();
            let td: i64 = deltas.iter().map(|d| d.delta_down).sum();
            s.traffic_history.push((tu, td));
            if s.traffic_history.len() > 60 { s.traffic_history.remove(0); }
            s.last_sync_time = Some(chrono::Local::now());
            s.grpc_connected = true;
        }
        TrafficEvent::GrpcConnected => { s.grpc_connected = true; }
        TrafficEvent::GrpcError(e) => {
            s.grpc_connected = false;
            s.push_log(format!("[ERROR] gRPC: {}", e));
            s.set_status(format!("gRPC 连接失败: {}", e), StatusLevel::Error);
        }
        TrafficEvent::QuotaAlert(n, p) => {
            s.push_log(format!("[WARN] {} 流量已用 {}%", n, p));
            s.set_status(format!("⚠ {} 用量 {}%", n, p), StatusLevel::Warn);
        }
        TrafficEvent::AutoControl(c) => {
            for item in c { s.push_log(format!("[INFO] 自动控制: {}", item)); }
        }
    }
}

fn handle_key(
    s: &mut AppState,
    k: KeyEvent,
    pool: Arc<sqlx::SqlitePool>,
    cfg: Arc<AppConfig>,
    ui_tx: mpsc::Sender<UiEvent>,
) {
    let len = s.users.len();
    match k.code {
        KeyCode::Tab       => s.next_page(),
        KeyCode::Char('1') => s.page = Page::Dashboard,
        KeyCode::Char('2') => s.page = Page::Users,
        KeyCode::Char('3') => s.page = Page::Nodes,
        KeyCode::Char('4') => s.page = Page::Logs,
        KeyCode::Up   | KeyCode::Char('k') => s.user_table.prev(len),
        KeyCode::Down | KeyCode::Char('j') => s.user_table.next(len),
        KeyCode::Esc       => s.status_msg = None,
        KeyCode::Char('t') => {
            if let Some(name) = s.selected_user().map(|u| u.name.clone()) {
                spawn_toggle(pool, cfg, ui_tx, name);
            }
        }
        KeyCode::Char('r') => {
            if let Some(name) = s.selected_user().map(|u| u.name.clone()) {
                spawn_reset(pool, cfg, ui_tx, name);
            }
        }
        KeyCode::Char('s') => {
            if let Some(name) = s.selected_user().map(|u| u.name.clone()) {
                spawn_export(cfg, ui_tx, name);
            }
        }
        KeyCode::Char('R') => spawn_refresh(pool, cfg, ui_tx),
        KeyCode::Char('c') => spawn_check(cfg, ui_tx),
        _ => {}
    }
}

fn spawn_toggle(pool: Arc<sqlx::SqlitePool>, cfg: Arc<AppConfig>, tx: mpsc::Sender<UiEvent>, name: String) {
    tokio::spawn(async move {
        match crate::service::user_service::toggle_user(&pool, &name).await {
            Ok(enabled) => {
                if let Err(e) = crate::apply_runtime_changes(&pool, &cfg).await {
                    let _ = tx.send(UiEvent::Status {
                        msg: format!("配置同步失败: {}", e),
                        level: StatusLevel::Error,
                    }).await;
                    return;
                }
                let _ = tx.send(UiEvent::UserEnabled { name, enabled }).await;
            }
            Err(e) => {
                let _ = tx.send(UiEvent::Status {
                    msg: format!("切换失败: {}", e),
                    level: StatusLevel::Error,
                }).await;
            }
        }
    });
}

fn spawn_reset(pool: Arc<sqlx::SqlitePool>, cfg: Arc<AppConfig>, tx: mpsc::Sender<UiEvent>, name: String) {
    tokio::spawn(async move {
        if let Err(e) = crate::service::user_service::reset_traffic(&pool, &name).await {
            let _ = tx.send(UiEvent::Status {
                msg: format!("重置失败: {}", e),
                level: StatusLevel::Error,
            }).await;
            return;
        }
        if let Err(e) = crate::apply_runtime_changes(&pool, &cfg).await {
            let _ = tx.send(UiEvent::Status {
                msg: format!("配置同步失败: {}", e),
                level: StatusLevel::Error,
            }).await;
            return;
        }
        let _ = tx.send(UiEvent::TrafficReset { name }).await;
    });
}

fn spawn_export(cfg: Arc<AppConfig>, tx: mpsc::Sender<UiEvent>, name: String) {
    tokio::spawn(async move {
        let cfg_json = match crate::core::config::load(&cfg.singbox.config_path) {
            Ok(v) => v,
            Err(e) => {
                let _ = tx.send(UiEvent::Status { msg: format!("读取配置失败: {}", e), level: StatusLevel::Error }).await;
                return;
            }
        };
        let server = crate::service::node_service::get_server_ip().await;
        match crate::service::sub_service::generate_links(&cfg_json, &name, &server) {
            Ok(links) if !links.is_empty() => {
                let text = crate::service::sub_service::generate_subscription(&links);
                let _ = tx.send(UiEvent::SubscriptionExported { name, text }).await;
            }
            Ok(_) => {
                let _ = tx.send(UiEvent::Status { msg: format!("{} 无可用订阅", name), level: StatusLevel::Warn }).await;
            }
            Err(e) => {
                let _ = tx.send(UiEvent::Status { msg: format!("导出失败: {}", e), level: StatusLevel::Error }).await;
            }
        }
    });
}

fn spawn_refresh(pool: Arc<sqlx::SqlitePool>, cfg: Arc<AppConfig>, tx: mpsc::Sender<UiEvent>) {
    tokio::spawn(async move {
        if let Ok(users) = crate::service::user_service::list_users(&pool).await {
            let _ = tx.send(UiEvent::UsersRefreshed(users)).await;
        }
        if let Ok(v) = crate::core::config::load(&cfg.singbox.config_path) {
            let _ = tx.send(UiEvent::NodesRefreshed(crate::service::node_service::list_nodes(&v))).await;
        }
        let proc = crate::core::singbox::SingboxProcess::new(&cfg.singbox.binary_path, &cfg.singbox.config_path);
        let _ = tx.send(UiEvent::SingboxRunning(proc.is_running())).await;
        let _ = tx.send(UiEvent::Status { msg: "已刷新".into(), level: StatusLevel::Warn }).await;
    });
}

fn spawn_check(cfg: Arc<AppConfig>, tx: mpsc::Sender<UiEvent>) {
    tokio::spawn(async move {
        let proc = crate::core::singbox::SingboxProcess::new(&cfg.singbox.binary_path, &cfg.singbox.config_path);
        let (msg, level) = match proc.check_config() {
            Ok(()) => ("sing-box 配置校验通过".to_string(), StatusLevel::Warn),
            Err(e) => (format!("配置校验失败: {}", e), StatusLevel::Error),
        };
        let _ = tx.send(UiEvent::Status { msg, level }).await;
    });
}
