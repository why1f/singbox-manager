use crate::model::{node::InboundNode, user::User};
use crate::tui::forms::Modal;

/// 需要主 loop 挂起 TUI、让外部命令接管 TTY 的动作。按键 handler 只设置，主 loop 统一执行。
#[derive(Debug, Clone, Copy)]
pub enum ExternalCmd {
    /// curl -fsSL install-release.sh | sudo bash
    SelfUpdate,
    /// $EDITOR /etc/sing-box/config.json → 退出后 sing-box check + reload
    EditSingboxConfig,
    /// journalctl -u sing-box -f（Ctrl-C 退出）
    TailSingboxLog,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Page { Dashboard, Users, Nodes, Logs, Kernel, Nginx }
impl Page {
    pub fn index(&self) -> usize {
        match self {
            Page::Dashboard => 0, Page::Users => 1, Page::Nodes => 2,
            Page::Logs => 3, Page::Kernel => 4, Page::Nginx => 5,
        }
    }
}
#[derive(Debug, Clone)]
pub enum StatusLevel { Warn, Error }

#[derive(Default)]
pub struct TableState { pub selected: usize }
impl TableState {
    pub fn next(&mut self, len: usize) { if len>0 { self.selected=(self.selected+1)%len; } }
    pub fn prev(&mut self, len: usize) { if len>0 { self.selected=if self.selected==0{len-1}else{self.selected-1}; } }
    pub fn clamp(&mut self, len: usize) {
        if len == 0 { self.selected = 0; return; }
        if self.selected >= len { self.selected = len - 1; }
    }
}

pub struct AppState {
    pub page: Page,
    pub users: Vec<User>,
    pub nodes: Vec<InboundNode>,
    pub singbox_running: Option<bool>,
    pub grpc_connected: bool,
    pub last_sync_time: Option<chrono::DateTime<chrono::Local>>,
    pub log_lines: Vec<String>,
    pub status_msg: Option<(String, StatusLevel)>,
    pub status_set_at: Option<std::time::Instant>,
    pub user_table: TableState,
    pub node_table: TableState,
    pub traffic_history: Vec<(i64,i64)>,
    pub uptime_secs: u64,
    pub last_subscription: Option<String>,
    pub modal: Option<Modal>,
    pub kernel: Option<crate::core::singbox::KernelStatus>,
    pub kernel_busy: Option<&'static str>,
    pub nginx: Option<crate::core::nginx::NginxStatus>,
    pub nginx_busy: Option<&'static str>,
    pub nginx_public_base: Option<String>,
    pub sub_public_base: Option<String>,
    // 系统指标历史（TUI 仪表盘曲线）
    pub cpu_history: Vec<u8>,        // 0-100
    pub net_rx_history: Vec<u64>,    // 每秒新增字节
    pub net_tx_history: Vec<u64>,
    pub dashboard_user_scroll: usize,
    pub dashboard_node_scroll: usize,
    /// 按键处理设置后，主 loop 下一次 iteration 会挂起 TUI 执行它然后清空
    pub pending_cmd: Option<ExternalCmd>,
}

impl Default for AppState {
    fn default() -> Self { Self::new() }
}

impl AppState {
    pub fn new() -> Self {
        Self {
            page: Page::Dashboard,
            users: vec![], nodes: vec![],
            singbox_running: None, grpc_connected: false,
            last_sync_time: None, log_lines: vec![], status_msg: None,
            status_set_at: None,
            user_table: TableState::default(),
            node_table: TableState::default(),
            traffic_history: vec![],
            uptime_secs: 0, last_subscription: None,
            modal: None,
            kernel: None,
            kernel_busy: None,
            nginx: None,
            nginx_busy: None,
            nginx_public_base: None,
            sub_public_base: None,
            cpu_history: Vec::new(),
            net_rx_history: Vec::new(),
            net_tx_history: Vec::new(),
            dashboard_user_scroll: 0,
            dashboard_node_scroll: 0,
            pending_cmd: None,
        }
    }
    pub fn selected_user(&self) -> Option<&User> {
        self.users.get(self.user_table.selected)
    }
    pub fn selected_node(&self) -> Option<&InboundNode> {
        self.nodes.get(self.node_table.selected)
    }
    pub fn push_log(&mut self, s: String) {
        self.log_lines.push(s);
        if self.log_lines.len()>500 { self.log_lines.drain(0..100); }
    }
    pub fn set_status(&mut self, msg: impl Into<String>, level: StatusLevel) {
        self.status_msg = Some((msg.into(), level));
        self.status_set_at = Some(std::time::Instant::now());
    }
    /// 自动清除过期状态（5s）；调用方每次 draw 前跑一下
    pub fn tick_status(&mut self) {
        if let Some(t) = self.status_set_at {
            if t.elapsed() >= std::time::Duration::from_secs(5) {
                self.status_msg = None;
                self.status_set_at = None;
            }
        }
    }
    pub fn next_page(&mut self) {
        self.page = match self.page {
            Page::Dashboard => Page::Users,
            Page::Users     => Page::Nodes,
            Page::Nodes     => Page::Logs,
            Page::Logs      => Page::Kernel,
            Page::Kernel    => Page::Nginx,
            Page::Nginx     => Page::Dashboard,
        };
    }
}
