use crate::model::{node::InboundNode, user::User};

#[derive(Debug, Clone, PartialEq)]
pub enum Page { Dashboard, Users, Nodes, Logs }
impl Page {
    pub fn index(&self) -> usize {
        match self { Page::Dashboard=>0, Page::Users=>1, Page::Nodes=>2, Page::Logs=>3 }
    }
}
#[derive(Debug, Clone)]
pub enum StatusLevel { Warn, Error }

#[derive(Default)]
pub struct UserTableState { pub selected: usize }
impl UserTableState {
    pub fn next(&mut self, len: usize) { if len>0 { self.selected=(self.selected+1)%len; } }
    pub fn prev(&mut self, len: usize) { if len>0 { self.selected=if self.selected==0{len-1}else{self.selected-1}; } }
}

pub struct AppState {
    pub page: Page, pub users: Vec<User>, pub nodes: Vec<InboundNode>,
    pub singbox_running: Option<bool>, pub grpc_connected: bool,
    pub last_sync_time: Option<chrono::DateTime<chrono::Local>>,
    pub log_lines: Vec<String>, pub status_msg: Option<(String, StatusLevel)>,
    pub user_table: UserTableState, pub traffic_history: Vec<(i64,i64)>,
    pub uptime_secs: u64, pub last_subscription: Option<String>,
}
impl Default for AppState {
    fn default() -> Self { Self::new() }
}

impl AppState {
    pub fn new() -> Self {
        Self { page:Page::Dashboard, users:vec![], nodes:vec![], singbox_running:None,
               grpc_connected:false, last_sync_time:None, log_lines:vec![], status_msg:None,
               user_table:Default::default(), traffic_history:vec![], uptime_secs:0, last_subscription:None }
    }
    pub fn selected_user(&self) -> Option<&User> {
        self.users.get(self.user_table.selected)
    }
    pub fn push_log(&mut self, s: String) {
        self.log_lines.push(s);
        if self.log_lines.len()>500 { self.log_lines.drain(0..100); }
    }
    pub fn set_status(&mut self, msg: impl Into<String>, level: StatusLevel) {
        self.status_msg = Some((msg.into(), level));
    }
    pub fn next_page(&mut self) {
        self.page = match self.page {
            Page::Dashboard=>Page::Users, Page::Users=>Page::Nodes,
            Page::Nodes=>Page::Logs,      Page::Logs=>Page::Dashboard,
        };
    }
}
