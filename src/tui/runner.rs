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
    tui::{
        app::{AppState, Page, StatusLevel},
        forms::{Modal, ModalAction, NodeForm, UserForm},
        pages, widgets,
    },
};

#[derive(Debug)]
pub enum UiEvent {
    UsersRefreshed(Vec<User>),
    NodesRefreshed(Vec<InboundNode>),
    SingboxRunning(Option<bool>),
    UserEnabled { name: String, enabled: bool },
    TrafficReset { name: String },
    SubscriptionExported { name: String, text: String },
    Status { msg: String, level: StatusLevel },
    KernelStatus(crate::core::singbox::KernelStatus),
    KernelBusy(Option<&'static str>),
    NginxStatus(crate::core::nginx::NginxStatus),
    NginxBusy(Option<&'static str>),
    SysMetrics { cpu: u8, rx: u64, tx: u64 },
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
    spawn_kernel_refresh(ui_tx.clone());
    spawn_nginx_refresh(ui_tx.clone(), cfg.clone());
    spawn_sys_sampler(ui_tx.clone());

    // 记录 public_base 给 nginx 页显示
    s.nginx_public_base = if cfg.subscription.public_base.is_empty() {
        None
    } else { Some(cfg.subscription.public_base.clone()) };
    s.sub_public_base = s.nginx_public_base.clone();

    loop {
        s.tick_status();
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
                Page::Kernel    => pages::kernel::render(f, c[1], s),
                Page::Nginx     => pages::nginx::render(f, c[1], s),
            }
            widgets::status_bar::render(f, c[2], s);
            if let Some(modal) = &s.modal {
                crate::tui::forms::render(f, area, modal);
            }
        })?;

        tokio::select! {
            biased;
            _ = tokio::time::sleep(Duration::from_millis(50)) => {
                if event::poll(Duration::from_millis(0))? {
                    if let CE::Key(k) = event::read()? {
                        if k.kind == KeyEventKind::Press {
                            if s.modal.is_some() {
                                if handle_modal_key(s, k, pool.clone(), cfg.clone(), ui_tx.clone()) {
                                    return Ok(());
                                }
                            } else {
                                if is_quit(&k) { return Ok(()); }
                                handle_page_key(s, k, pool.clone(), cfg.clone(), ui_tx.clone());
                            }
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
        UiEvent::UsersRefreshed(u) => { s.users = u; s.user_table.clamp(s.users.len()); }
        UiEvent::NodesRefreshed(n) => { s.nodes = n; s.node_table.clamp(s.nodes.len()); }
        UiEvent::SingboxRunning(v) => s.singbox_running = v,
        UiEvent::UserEnabled { name, enabled } => {
            if let Some(u) = s.users.iter_mut().find(|u| u.name == name) { u.enabled = enabled; }
            s.push_log(format!("[INFO] {} 已{}", name, if enabled { "启用" } else { "禁用" }));
        }
        UiEvent::TrafficReset { name } => {
            if let Some(u) = s.users.iter_mut().find(|u| u.name == name) {
                u.used_up_bytes = 0; u.used_down_bytes = 0;
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
        UiEvent::KernelStatus(k) => {
            s.singbox_running = k.running;
            s.kernel = Some(k);
        }
        UiEvent::KernelBusy(op) => s.kernel_busy = op,
        UiEvent::NginxStatus(n) => s.nginx = Some(n),
        UiEvent::NginxBusy(op)  => s.nginx_busy = op,
        UiEvent::SysMetrics { cpu, rx, tx } => {
            s.cpu_history.push(cpu);
            if s.cpu_history.len() > 60 { s.cpu_history.remove(0); }
            s.net_rx_history.push(rx);
            if s.net_rx_history.len() > 60 { s.net_rx_history.remove(0); }
            s.net_tx_history.push(tx);
            if s.net_tx_history.len() > 60 { s.net_tx_history.remove(0); }
        }
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

fn handle_modal_key(
    s: &mut AppState,
    k: KeyEvent,
    pool: Arc<sqlx::SqlitePool>,
    cfg: Arc<AppConfig>,
    ui_tx: mpsc::Sender<UiEvent>,
) -> bool {
    use crossterm::event::KeyModifiers;
    if matches!(k.code, KeyCode::Char('c')) && k.modifiers == KeyModifiers::CONTROL {
        return true;
    }
    let Some(modal) = s.modal.as_mut() else { return false; };
    match modal.handle(k) {
        ModalAction::None => {}
        ModalAction::Close => s.modal = None,
        ModalAction::SubmitUser { name, quota, reset_day, expire } => {
            s.modal = None;
            spawn_add_user(pool, cfg, ui_tx, name, quota, reset_day, expire);
        }
        ModalAction::SubmitUserEdit { name, quota, reset_day, expire } => {
            s.modal = None;
            spawn_edit_user(pool, cfg, ui_tx, name, quota, reset_day, expire);
        }
        ModalAction::SubmitNode { tag, protocol, port, server_name, path, port_reuse } => {
            s.modal = None;
            spawn_add_node(cfg, ui_tx, tag, protocol, port, server_name, path, port_reuse);
        }
        ModalAction::SubmitNodeEdit { tag, port, server_name, path, port_reuse } => {
            s.modal = None;
            spawn_edit_node(cfg, ui_tx, tag, port, server_name, path, port_reuse);
        }
        ModalAction::DeleteUser(name) => {
            s.modal = None;
            spawn_delete_user(pool, cfg, ui_tx, name);
        }
        ModalAction::DeleteNode(tag) => {
            s.modal = None;
            spawn_delete_node(cfg, ui_tx, tag);
        }
        ModalAction::SaveNodePicker { user, all, tags } => {
            s.modal = None;
            spawn_save_nodes(pool, cfg, ui_tx, user, all, tags);
        }
        ModalAction::RegenToken(name) => {
            s.modal = None;
            spawn_regen_token(pool, ui_tx, name);
        }
        ModalAction::RevokeToken(name) => {
            s.modal = None;
            spawn_revoke_token(pool, ui_tx, name);
        }
    }
    false
}

fn handle_page_key(
    s: &mut AppState,
    k: KeyEvent,
    pool: Arc<sqlx::SqlitePool>,
    cfg: Arc<AppConfig>,
    ui_tx: mpsc::Sender<UiEvent>,
) {
    // 内核页/Nginx 页优先处理，避免与用户/节点页的 r/t/s 等按键冲突
    if s.page == Page::Kernel {
        handle_kernel_key(s, k, ui_tx, cfg);
        return;
    }
    if s.page == Page::Nginx {
        handle_nginx_key(s, k, ui_tx, cfg);
        return;
    }
    match k.code {
        KeyCode::Tab       => { s.next_page(); maybe_refresh_kernel(s, &ui_tx); }
        KeyCode::Char('1') => s.page = Page::Dashboard,
        KeyCode::Char('2') => s.page = Page::Users,
        KeyCode::Char('3') => s.page = Page::Nodes,
        KeyCode::Char('4') => s.page = Page::Logs,
        KeyCode::Char('5') => { s.page = Page::Kernel; maybe_refresh_kernel(s, &ui_tx); }
        KeyCode::Char('6') => { s.page = Page::Nginx;  maybe_refresh_nginx(s, &ui_tx, cfg.clone()); }
        KeyCode::Esc       => s.status_msg = None,
        KeyCode::Up   | KeyCode::Char('k') => match s.page {
            Page::Users => s.user_table.prev(s.users.len()),
            Page::Nodes => s.node_table.prev(s.nodes.len()),
            _ => {}
        },
        KeyCode::Down | KeyCode::Char('j') => match s.page {
            Page::Users => s.user_table.next(s.users.len()),
            Page::Nodes => s.node_table.next(s.nodes.len()),
            _ => {}
        },
        KeyCode::Char('a') => match s.page {
            Page::Users => s.modal = Some(Modal::AddUser(UserForm::default())),
            Page::Nodes => s.modal = Some(Modal::AddNode(NodeForm {
                port: "443".into(),
                ..NodeForm::default()
            })),
            _ => {}
        },
        KeyCode::Char('E') => match s.page {
            Page::Users => if let Some(u) = s.selected_user() {
                s.modal = Some(Modal::EditUser(crate::tui::forms::UserEditForm {
                    name: u.name.clone(),
                    quota: if u.quota_gb <= 0.0 { String::new() } else { format!("{}", u.quota_gb) },
                    reset_day: if u.reset_day == 0 { String::new() } else { u.reset_day.to_string() },
                    expire: u.expire_at.clone(),
                    ..Default::default()
                }));
            },
            Page::Nodes => if let Some(n) = s.selected_node() {
                let port_reuse = crate::core::config::get_node_meta(&n.tag)
                    .map(|m| m.port_reuse).unwrap_or(false);
                s.modal = Some(Modal::EditNode(crate::tui::forms::NodeEditForm {
                    tag: n.tag.clone(),
                    protocol: n.protocol.to_string(),
                    port: n.listen_port.to_string(),
                    server_name: String::new(),
                    path: String::new(),
                    port_reuse,
                    ..Default::default()
                }));
            },
            _ => {}
        },
        KeyCode::Char('d') => match s.page {
            Page::Users => if let Some(name) = s.selected_user().map(|u| u.name.clone()) {
                s.modal = Some(Modal::ConfirmDeleteUser(name));
            },
            Page::Nodes => if let Some(tag) = s.selected_node().map(|n| n.tag.clone()) {
                s.modal = Some(Modal::ConfirmDeleteNode(tag));
            },
            _ => {}
        },
        KeyCode::Char('n') if s.page == Page::Users => {
            if let Some(u) = s.selected_user() {
                let name = u.name.clone();
                let all = u.allow_all_nodes;
                let allowed = u.allowed_tags();
                let tags: Vec<String> = s.nodes.iter().map(|n| n.tag.clone()).collect();
                let checked: Vec<bool> = tags.iter().map(|t| allowed.iter().any(|a| a == t)).collect();
                s.modal = Some(Modal::NodePicker(crate::tui::forms::NodePicker {
                    user: name, tags, checked, cursor: 0, all,
                }));
            }
        },
        KeyCode::Char('t') => if let Some(name) = s.selected_user().map(|u| u.name.clone()) {
            spawn_toggle(pool, cfg, ui_tx, name);
        },
        KeyCode::Char('r') => if let Some(name) = s.selected_user().map(|u| u.name.clone()) {
            spawn_reset(pool, cfg, ui_tx, name);
        },
        KeyCode::Char('s') => if let Some(name) = s.selected_user().map(|u| u.name.clone()) {
            spawn_export(cfg, ui_tx, name);
        },
        KeyCode::Char('u') => if let Some(u) = s.selected_user() {
            if u.sub_token.is_empty() {
                s.set_status(format!("{} 无订阅 token", u.name), StatusLevel::Warn);
            } else if let Some(base) = s.sub_public_base.clone() {
                let base = base.trim_end_matches('/').to_string();
                s.modal = Some(Modal::SubUrl {
                    name:    u.name.clone(),
                    singbox: format!("{}/sub/{}", base, u.sub_token),
                    mihomo:  format!("{}/sub/{}?type=clash", base, u.sub_token),
                });
            } else {
                s.set_status(
                    format!("需在 config.toml 里填 [subscription].public_base；当前 token: {}", u.sub_token),
                    StatusLevel::Warn,
                );
            }
        },
        KeyCode::Char('R') => spawn_refresh(pool, cfg, ui_tx),
        KeyCode::Char('c') => spawn_check(cfg, ui_tx),
        KeyCode::Char('T') if s.page == Page::Users => {
            if let Some(u) = s.selected_user() {
                s.modal = Some(Modal::TokenManage {
                    name: u.name.clone(),
                    has_token: !u.sub_token.is_empty(),
                });
            }
        }
        _ => {}
    }
}

fn handle_kernel_key(s: &mut AppState, k: KeyEvent, ui_tx: mpsc::Sender<UiEvent>, cfg: Arc<AppConfig>) {
    match k.code {
        KeyCode::Tab       => { s.next_page(); maybe_refresh_kernel(s, &ui_tx); }
        KeyCode::Char('1') => s.page = Page::Dashboard,
        KeyCode::Char('2') => s.page = Page::Users,
        KeyCode::Char('3') => s.page = Page::Nodes,
        KeyCode::Char('4') => s.page = Page::Logs,
        KeyCode::Char('5') => { /* 已在本页 */ }
        KeyCode::Esc       => s.status_msg = None,
        KeyCode::Char('R') => spawn_kernel_refresh(ui_tx),
        _ if s.kernel_busy.is_some() => {
            s.set_status("正在执行上一操作，请稍候", StatusLevel::Warn);
        }
        KeyCode::Char('i') => spawn_kernel_op(ui_tx, "安装官方版 sing-box", crate::core::singbox::install_latest),
        KeyCode::Char('v') => spawn_kernel_install_v2rayapi(ui_tx, cfg.kernel.update_repo.clone()),
        KeyCode::Char('u') => spawn_kernel_op(ui_tx, "卸载 sing-box",       crate::core::singbox::uninstall),
        KeyCode::Char('s') => spawn_kernel_op(ui_tx, "启动 sing-box",       crate::core::singbox::start),
        KeyCode::Char('S') => spawn_kernel_op(ui_tx, "停止 sing-box",       crate::core::singbox::stop),
        KeyCode::Char('x') => spawn_kernel_op(ui_tx, "重启 sing-box",       crate::core::singbox::restart),
        KeyCode::Char('e') => spawn_kernel_op(ui_tx, "启用开机自启",        crate::core::singbox::enable),
        KeyCode::Char('d') => spawn_kernel_op(ui_tx, "关闭开机自启",        crate::core::singbox::disable),
        _ => {}
    }
}

fn maybe_refresh_kernel(s: &AppState, ui_tx: &mpsc::Sender<UiEvent>) {
    if s.page == Page::Kernel {
        spawn_kernel_refresh(ui_tx.clone());
    }
}

fn maybe_refresh_nginx(s: &AppState, ui_tx: &mpsc::Sender<UiEvent>, cfg: Arc<AppConfig>) {
    if s.page == Page::Nginx {
        spawn_nginx_refresh(ui_tx.clone(), cfg);
    }
}

fn handle_nginx_key(s: &mut AppState, k: KeyEvent, ui_tx: mpsc::Sender<UiEvent>, cfg: Arc<AppConfig>) {
    match k.code {
        KeyCode::Tab       => { s.next_page(); maybe_refresh_kernel(s, &ui_tx); }
        KeyCode::Char('1') => s.page = Page::Dashboard,
        KeyCode::Char('2') => s.page = Page::Users,
        KeyCode::Char('3') => s.page = Page::Nodes,
        KeyCode::Char('4') => s.page = Page::Logs,
        KeyCode::Char('5') => { s.page = Page::Kernel; maybe_refresh_kernel(s, &ui_tx); }
        KeyCode::Char('6') => { /* 已在本页 */ }
        KeyCode::Esc       => s.status_msg = None,
        KeyCode::Char('R') => spawn_nginx_refresh(ui_tx, cfg),
        _ if s.nginx_busy.is_some() => {
            s.set_status("正在执行上一操作，请稍候", StatusLevel::Warn);
        }
        KeyCode::Char('i') => spawn_nginx_op(ui_tx, cfg, "安装 nginx",        crate::core::nginx::install_via_pkg),
        KeyCode::Char('s') => spawn_nginx_op(ui_tx, cfg, "启动 nginx",        crate::core::nginx::start),
        KeyCode::Char('S') => spawn_nginx_op(ui_tx, cfg, "停止 nginx",        crate::core::nginx::stop),
        KeyCode::Char('x') => spawn_nginx_op(ui_tx, cfg, "重启 nginx",        crate::core::nginx::restart),
        KeyCode::Char('l') => spawn_nginx_op(ui_tx, cfg, "reload nginx",      crate::core::nginx::reload),
        KeyCode::Char('e') => spawn_nginx_op(ui_tx, cfg, "nginx 开机自启",    crate::core::nginx::enable),
        KeyCode::Char('d') => spawn_nginx_op(ui_tx, cfg, "nginx 取消自启",    crate::core::nginx::disable),
        KeyCode::Char('t') => spawn_nginx_test(ui_tx),
        KeyCode::Char('g') => spawn_nginx_genconf(ui_tx, cfg),
        _ => {}
    }
}

fn spawn_nginx_refresh(tx: mpsc::Sender<UiEvent>, cfg: Arc<AppConfig>) {
    tokio::spawn(async move {
        let conf_path = cfg.subscription.nginx_conf.clone();
        let status = tokio::task::spawn_blocking(move || crate::core::nginx::status(&conf_path)).await
            .unwrap_or(crate::core::nginx::NginxStatus {
                installed: false, running: None, enabled: false,
                version: None, binary_path: None, conf_exists: false,
            });
        let _ = tx.send(UiEvent::NginxStatus(status)).await;
    });
}

fn spawn_nginx_op<F>(tx: mpsc::Sender<UiEvent>, cfg: Arc<AppConfig>, label: &'static str, op: F)
where F: FnOnce() -> anyhow::Result<()> + Send + 'static {
    tokio::spawn(async move {
        let _ = tx.send(UiEvent::NginxBusy(Some(label))).await;
        let result = tokio::task::spawn_blocking(op).await;
        let _ = tx.send(UiEvent::NginxBusy(None)).await;
        let (msg, level) = match result {
            Ok(Ok(()))  => (format!("{} 成功", label), StatusLevel::Warn),
            Ok(Err(e))  => (format!("{} 失败: {}", label, e), StatusLevel::Error),
            Err(e)      => (format!("{} 中断: {}", label, e), StatusLevel::Error),
        };
        let _ = tx.send(UiEvent::Status { msg, level }).await;
        // 刷新状态
        let conf_path = cfg.subscription.nginx_conf.clone();
        let status = tokio::task::spawn_blocking(move || crate::core::nginx::status(&conf_path)).await
            .unwrap_or(crate::core::nginx::NginxStatus {
                installed: false, running: None, enabled: false,
                version: None, binary_path: None, conf_exists: false,
            });
        let _ = tx.send(UiEvent::NginxStatus(status)).await;
    });
}

fn spawn_nginx_test(tx: mpsc::Sender<UiEvent>) {
    tokio::spawn(async move {
        let _ = tx.send(UiEvent::NginxBusy(Some("nginx -t"))).await;
        let result = tokio::task::spawn_blocking(crate::core::nginx::test_config).await;
        let _ = tx.send(UiEvent::NginxBusy(None)).await;
        let (msg, level) = match result {
            Ok(Ok(out)) => {
                let first = out.lines().filter(|l| !l.is_empty()).take(2).collect::<Vec<_>>().join(" | ");
                (format!("nginx -t 通过: {}", first), StatusLevel::Warn)
            }
            Ok(Err(e))  => (format!("nginx -t 失败: {}", e), StatusLevel::Error),
            Err(e)      => (format!("nginx -t 中断: {}", e), StatusLevel::Error),
        };
        let _ = tx.send(UiEvent::Status { msg, level }).await;
    });
}

fn spawn_nginx_genconf(tx: mpsc::Sender<UiEvent>, cfg: Arc<AppConfig>) {
    tokio::spawn(async move {
        let res = tokio::task::spawn_blocking(move || {
            crate::core::nginx::generate_conf(
                &cfg.subscription.nginx_conf,
                &cfg.subscription.public_base,
                &cfg.subscription.listen,
            )
        }).await;
        let (msg, level) = match res {
            Ok(Ok(())) => ("✓ 反代配置已生成；编辑证书路径后 [t] 检查 + [l] reload".into(), StatusLevel::Warn),
            Ok(Err(e)) => (format!("生成失败: {}", e), StatusLevel::Error),
            Err(e)     => (format!("任务中断: {}", e), StatusLevel::Error),
        };
        let _ = tx.send(UiEvent::Status { msg, level }).await;
    });
}

fn spawn_kernel_refresh(tx: mpsc::Sender<UiEvent>) {
    tokio::spawn(async move {
        let status = tokio::task::spawn_blocking(crate::core::singbox::status).await
            .unwrap_or(crate::core::singbox::KernelStatus {
                installed: false, running: None, enabled: false, version: None, binary_path: None,
            });
        let _ = tx.send(UiEvent::KernelStatus(status)).await;
    });
}

fn spawn_sys_sampler(tx: mpsc::Sender<UiEvent>) {
    tokio::spawn(async move {
        use crate::core::sysinfo;
        let mut iv = tokio::time::interval(std::time::Duration::from_secs(1));
        iv.tick().await;
        let mut prev_cpu = sysinfo::read_cpu();
        let mut prev_net = sysinfo::read_net();
        loop {
            iv.tick().await;
            let cur_cpu = sysinfo::read_cpu();
            let cur_net = sysinfo::read_net();
            let cpu = match (prev_cpu.as_ref(), cur_cpu.as_ref()) {
                (Some(p), Some(c)) => sysinfo::cpu_percent(p, c),
                _ => 0,
            };
            let (rx, tx_bytes) = match (prev_net, cur_net) {
                (Some(p), Some(c)) => (c.0.saturating_sub(p.0), c.1.saturating_sub(p.1)),
                _ => (0, 0),
            };
            if tx.send(UiEvent::SysMetrics { cpu, rx, tx: tx_bytes }).await.is_err() { break; }
            prev_cpu = cur_cpu;
            prev_net = cur_net;
        }
    });
}

fn spawn_kernel_op<F>(tx: mpsc::Sender<UiEvent>, label: &'static str, op: F)
where F: FnOnce() -> anyhow::Result<()> + Send + 'static {
    tokio::spawn(async move {
        let _ = tx.send(UiEvent::KernelBusy(Some(label))).await;
        let result = tokio::task::spawn_blocking(op).await;
        let _ = tx.send(UiEvent::KernelBusy(None)).await;
        match result {
            Ok(Ok(())) => {
                let _ = tx.send(UiEvent::Status { msg: format!("{} 成功", label), level: StatusLevel::Warn }).await;
            }
            Ok(Err(e)) => {
                let _ = tx.send(UiEvent::Status { msg: format!("{} 失败: {}", label, e), level: StatusLevel::Error }).await;
            }
            Err(e) => {
                let _ = tx.send(UiEvent::Status { msg: format!("{} 中断: {}", label, e), level: StatusLevel::Error }).await;
            }
        }
        // systemctl 返回后进程可能还没被 pgrep 看到，延迟 + 轮询
        refresh_kernel_status_stable(&tx).await;
    });
}

fn spawn_kernel_install_v2rayapi(tx: mpsc::Sender<UiEvent>, repo: String) {
    tokio::spawn(async move {
        let label = "安装 v2ray_api 版 sing-box";
        let _ = tx.send(UiEvent::KernelBusy(Some(label))).await;
        let result = crate::core::singbox::install_v2rayapi(&repo).await;
        let _ = tx.send(UiEvent::KernelBusy(None)).await;
        match result {
            Ok(()) => {
                let _ = tx.send(UiEvent::Status { msg: format!("{} 成功（已自动 enable + restart）", label), level: StatusLevel::Warn }).await;
            }
            Err(e) => {
                let _ = tx.send(UiEvent::Status { msg: format!("{} 失败: {}", label, e), level: StatusLevel::Error }).await;
            }
        }
        refresh_kernel_status_stable(&tx).await;
    });
}

/// 轮询 3 次，每次间隔 500ms，取最后一次结果（避开 systemd 启动竞态）
async fn refresh_kernel_status_stable(tx: &mpsc::Sender<UiEvent>) {
    for i in 0..3 {
        if i > 0 { tokio::time::sleep(std::time::Duration::from_millis(500)).await; }
        let status = tokio::task::spawn_blocking(crate::core::singbox::status).await
            .unwrap_or(crate::core::singbox::KernelStatus {
                installed: false, running: None, enabled: false, version: None, binary_path: None,
            });
        let _ = tx.send(UiEvent::KernelStatus(status)).await;
    }
}

fn spawn_save_nodes(
    pool: Arc<sqlx::SqlitePool>, cfg: Arc<AppConfig>, tx: mpsc::Sender<UiEvent>,
    user: String, all: bool, tags: Vec<String>,
) {
    tokio::spawn(async move {
        let res = if all {
            crate::service::user_service::grant_all_nodes(&pool, &user).await
        } else {
            crate::service::user_service::set_allowed_tags(&pool, &user, &tags).await
        };
        if let Err(e) = res {
            let _ = tx.send(UiEvent::Status { msg: format!("保存失败: {}", e), level: StatusLevel::Error }).await;
            return;
        }
        if let Err(e) = crate::apply_runtime_changes(&pool, &cfg).await {
            let _ = tx.send(UiEvent::Status { msg: format!("配置同步失败: {}", e), level: StatusLevel::Error }).await;
            return;
        }
        if let Ok(users) = crate::service::user_service::list_users(&pool).await {
            let _ = tx.send(UiEvent::UsersRefreshed(users)).await;
        }
        let msg = if all { format!("{} 已恢复全部节点可用", user) }
                  else { format!("{} 已分配 {} 个节点", user, tags.len()) };
        let _ = tx.send(UiEvent::Status { msg, level: StatusLevel::Warn }).await;
    });
}

fn spawn_edit_user(
    pool: Arc<sqlx::SqlitePool>, cfg: Arc<AppConfig>, tx: mpsc::Sender<UiEvent>,
    name: String, quota: Option<f64>, reset_day: Option<i64>, expire: Option<String>,
) {
    tokio::spawn(async move {
        if let Err(e) = crate::service::user_service::update_package(
            &pool, &name, quota, reset_day, expire.as_deref()
        ).await {
            let _ = tx.send(UiEvent::Status { msg: format!("更新失败: {}", e), level: StatusLevel::Error }).await;
            return;
        }
        if let Err(e) = crate::apply_runtime_changes(&pool, &cfg).await {
            let _ = tx.send(UiEvent::Status { msg: format!("配置同步失败: {}", e), level: StatusLevel::Error }).await;
            return;
        }
        if let Ok(users) = crate::service::user_service::list_users(&pool).await {
            let _ = tx.send(UiEvent::UsersRefreshed(users)).await;
        }
        let _ = tx.send(UiEvent::Status { msg: format!("{} 已更新", name), level: StatusLevel::Warn }).await;
    });
}

fn spawn_edit_node(
    cfg: Arc<AppConfig>, tx: mpsc::Sender<UiEvent>,
    tag: String, port: Option<u16>, server_name: Option<String>, path: Option<String>,
    port_reuse: Option<bool>,
) {
    tokio::spawn(async move {
        let mut cfg_json = match crate::core::config::load(&cfg.singbox.config_path) {
            Ok(v) => v,
            Err(e) => {
                let _ = tx.send(UiEvent::Status { msg: format!("读取配置失败: {}", e), level: StatusLevel::Error }).await;
                return;
            }
        };
        if let Err(e) = crate::core::config::edit_node(&mut cfg_json, &tag, port, server_name, path, port_reuse) {
            let _ = tx.send(UiEvent::Status { msg: format!("编辑失败: {}", e), level: StatusLevel::Error }).await;
            return;
        }
        if let Err(e) = crate::core::config::save(&cfg.singbox.config_path, &cfg_json) {
            let _ = tx.send(UiEvent::Status { msg: format!("保存失败: {}", e), level: StatusLevel::Error }).await;
            return;
        }
        let proc = crate::core::singbox::SingboxProcess::new(&cfg.singbox.binary_path, &cfg.singbox.config_path);
        let check_msg = match proc.check_config() {
            Ok(()) => {
                if matches!(proc.is_running(), Some(true)) { let _ = proc.reload(); }
                None
            }
            Err(e) => Some(format!("sing-box 校验/reload 失败: {}（改动已保存）", e)),
        };
        if let Ok(v) = crate::core::config::load(&cfg.singbox.config_path) {
            let _ = tx.send(UiEvent::NodesRefreshed(crate::service::node_service::list_nodes(&v))).await;
        }
        let (msg, level) = match check_msg {
            Some(w) => (w, StatusLevel::Error),
            None    => (format!("节点 {} 已更新", tag), StatusLevel::Warn),
        };
        let _ = tx.send(UiEvent::Status { msg, level }).await;
    });
}

fn spawn_toggle(pool: Arc<sqlx::SqlitePool>, cfg: Arc<AppConfig>, tx: mpsc::Sender<UiEvent>, name: String) {
    tokio::spawn(async move {
        match crate::service::user_service::toggle_user(&pool, &name).await {
            Ok(enabled) => {
                if let Err(e) = crate::apply_runtime_changes(&pool, &cfg).await {
                    let _ = tx.send(UiEvent::Status { msg: format!("配置同步失败: {}", e), level: StatusLevel::Error }).await;
                    return;
                }
                let _ = tx.send(UiEvent::UserEnabled { name, enabled }).await;
            }
            Err(e) => {
                let _ = tx.send(UiEvent::Status { msg: format!("切换失败: {}", e), level: StatusLevel::Error }).await;
            }
        }
    });
}

fn spawn_regen_token(pool: Arc<sqlx::SqlitePool>, tx: mpsc::Sender<UiEvent>, name: String) {
    tokio::spawn(async move {
        match crate::service::user_service::regen_sub_token(&pool, &name).await {
            Ok(_) => {
                if let Ok(users) = crate::service::user_service::list_users(&pool).await {
                    let _ = tx.send(UiEvent::UsersRefreshed(users)).await;
                }
                let _ = tx.send(UiEvent::Status {
                    msg: format!("{} 的 token 已轮换；老 URL 立即失效", name),
                    level: StatusLevel::Warn,
                }).await;
            }
            Err(e) => {
                let _ = tx.send(UiEvent::Status { msg: format!("重置 token 失败: {}", e), level: StatusLevel::Error }).await;
            }
        }
    });
}

fn spawn_revoke_token(pool: Arc<sqlx::SqlitePool>, tx: mpsc::Sender<UiEvent>, name: String) {
    tokio::spawn(async move {
        match crate::service::user_service::revoke_sub_token(&pool, &name).await {
            Ok(()) => {
                if let Ok(users) = crate::service::user_service::list_users(&pool).await {
                    let _ = tx.send(UiEvent::UsersRefreshed(users)).await;
                }
                let _ = tx.send(UiEvent::Status {
                    msg: format!("{} 的订阅已关闭；再 [T][g] 可恢复", name),
                    level: StatusLevel::Warn,
                }).await;
            }
            Err(e) => {
                let _ = tx.send(UiEvent::Status { msg: format!("撤销 token 失败: {}", e), level: StatusLevel::Error }).await;
            }
        }
    });
}

fn spawn_reset(pool: Arc<sqlx::SqlitePool>, cfg: Arc<AppConfig>, tx: mpsc::Sender<UiEvent>, name: String) {
    tokio::spawn(async move {
        if let Err(e) = crate::service::user_service::reset_traffic(&pool, &name).await {
            let _ = tx.send(UiEvent::Status { msg: format!("重置失败: {}", e), level: StatusLevel::Error }).await;
            return;
        }
        if let Err(e) = crate::apply_runtime_changes(&pool, &cfg).await {
            let _ = tx.send(UiEvent::Status { msg: format!("配置同步失败: {}", e), level: StatusLevel::Error }).await;
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

fn spawn_add_user(
    pool: Arc<sqlx::SqlitePool>, cfg: Arc<AppConfig>, tx: mpsc::Sender<UiEvent>,
    name: String, quota: f64, reset_day: i64, expire: String,
) {
    tokio::spawn(async move {
        match crate::service::user_service::add_user(&pool, &name, quota, reset_day, &expire).await {
            Ok(_) => {
                if let Err(e) = crate::apply_runtime_changes(&pool, &cfg).await {
                    let _ = tx.send(UiEvent::Status { msg: format!("配置同步失败: {}", e), level: StatusLevel::Error }).await;
                }
                if let Ok(users) = crate::service::user_service::list_users(&pool).await {
                    let _ = tx.send(UiEvent::UsersRefreshed(users)).await;
                }
                let _ = tx.send(UiEvent::Status { msg: format!("已添加用户 {}", name), level: StatusLevel::Warn }).await;
            }
            Err(e) => {
                let _ = tx.send(UiEvent::Status { msg: format!("添加失败: {}", e), level: StatusLevel::Error }).await;
            }
        }
    });
}

fn spawn_delete_user(
    pool: Arc<sqlx::SqlitePool>, cfg: Arc<AppConfig>, tx: mpsc::Sender<UiEvent>, name: String,
) {
    tokio::spawn(async move {
        if let Err(e) = crate::service::user_service::delete_user(&pool, &name).await {
            let _ = tx.send(UiEvent::Status { msg: format!("删除失败: {}", e), level: StatusLevel::Error }).await;
            return;
        }
        if let Err(e) = crate::apply_runtime_changes(&pool, &cfg).await {
            let _ = tx.send(UiEvent::Status { msg: format!("配置同步失败: {}", e), level: StatusLevel::Error }).await;
        }
        if let Ok(users) = crate::service::user_service::list_users(&pool).await {
            let _ = tx.send(UiEvent::UsersRefreshed(users)).await;
        }
        let _ = tx.send(UiEvent::Status { msg: format!("已删除用户 {}", name), level: StatusLevel::Warn }).await;
    });
}

#[allow(clippy::too_many_arguments)]
fn spawn_add_node(
    cfg: Arc<AppConfig>, tx: mpsc::Sender<UiEvent>,
    tag: String, protocol: String, port: u16,
    server_name: Option<String>, path: Option<String>, port_reuse: bool,
) {
    tokio::spawn(async move {
        let p = match crate::model::node::Protocol::try_from(protocol.as_str()) {
            Ok(p) => p,
            Err(e) => {
                let _ = tx.send(UiEvent::Status { msg: format!("{}", e), level: StatusLevel::Error }).await;
                return;
            }
        };
        let req = crate::model::node::AddNodeRequest {
            tag: tag.clone(), protocol: p, listen_port: port, server_name, path, port_reuse,
        };
        let mut cfg_json = match crate::core::config::load(&cfg.singbox.config_path) {
            Ok(v) => v,
            Err(_) => serde_json::json!({ "inbounds": [], "outbounds": [] }),
        };
        let add_result = crate::core::config::add_node(&mut cfg_json, &req);
        let meta = match add_result {
            Ok(m) => m,
            Err(e) => {
                let _ = tx.send(UiEvent::Status { msg: format!("添加节点失败: {}", e), level: StatusLevel::Error }).await;
                return;
            }
        };
        if let Err(e) = crate::core::config::save(&cfg.singbox.config_path, &cfg_json) {
            let _ = tx.send(UiEvent::Status { msg: format!("保存配置失败: {}", e), level: StatusLevel::Error }).await;
            return;
        }
        // 无论校验/reload 是否失败，都刷新节点列表（节点已经写入 config）
        let proc = crate::core::singbox::SingboxProcess::new(&cfg.singbox.binary_path, &cfg.singbox.config_path);
        let check_msg = match proc.check_config() {
            Ok(()) => {
                if matches!(proc.is_running(), Some(true)) { let _ = proc.reload(); }
                None
            }
            Err(e) => Some(format!("sing-box 校验/reload 失败: {}（节点已保存到 config.json）", e)),
        };
        if let Ok(v) = crate::core::config::load(&cfg.singbox.config_path) {
            let _ = tx.send(UiEvent::NodesRefreshed(crate::service::node_service::list_nodes(&v))).await;
        }
        let done_msg = match meta {
            crate::core::config::AddNodeMeta::RealityKeys { public_key, short_id } => format!(
                "已添加节点 {}  |  reality 公钥: {}  short_id: {}",
                tag, public_key, short_id,
            ),
            crate::core::config::AddNodeMeta::Plain => format!("已添加节点 {}", tag),
        };
        let (msg, level) = match check_msg {
            Some(w) => (w, StatusLevel::Error),
            None    => (done_msg, StatusLevel::Warn),
        };
        let _ = tx.send(UiEvent::Status { msg, level }).await;
    });
}

fn spawn_delete_node(
    cfg: Arc<AppConfig>, tx: mpsc::Sender<UiEvent>, tag: String,
) {
    tokio::spawn(async move {
        let mut cfg_json = match crate::core::config::load(&cfg.singbox.config_path) {
            Ok(v) => v,
            Err(e) => {
                let _ = tx.send(UiEvent::Status { msg: format!("读取配置失败: {}", e), level: StatusLevel::Error }).await;
                return;
            }
        };
        if !crate::core::config::remove_node(&mut cfg_json, &tag) {
            let _ = tx.send(UiEvent::Status { msg: format!("未找到节点 {}", tag), level: StatusLevel::Warn }).await;
            return;
        }
        if let Err(e) = crate::core::config::save(&cfg.singbox.config_path, &cfg_json) {
            let _ = tx.send(UiEvent::Status { msg: format!("保存失败: {}", e), level: StatusLevel::Error }).await;
            return;
        }
        let proc = crate::core::singbox::SingboxProcess::new(&cfg.singbox.binary_path, &cfg.singbox.config_path);
        let check_msg = match proc.check_config() {
            Ok(()) => {
                if matches!(proc.is_running(), Some(true)) { let _ = proc.reload(); }
                None
            }
            Err(e) => Some(format!("sing-box 校验/reload 失败: {}（节点已从 config.json 移除）", e)),
        };
        if let Ok(v) = crate::core::config::load(&cfg.singbox.config_path) {
            let _ = tx.send(UiEvent::NodesRefreshed(crate::service::node_service::list_nodes(&v))).await;
        }
        let (msg, level) = match check_msg {
            Some(w) => (w, StatusLevel::Error),
            None    => (format!("已删除节点 {}", tag), StatusLevel::Warn),
        };
        let _ = tx.send(UiEvent::Status { msg, level }).await;
    });
}
