use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Protocol {
    VlessReality,
    VlessWs,
    VmessWs,
    Shadowsocks,
    Trojan,
    Tuic,
    Anytls,
    Hysteria2,
    Unknown,
}
impl std::fmt::Display for Protocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Protocol::VlessReality => "vless-reality",
                Protocol::VlessWs => "vless-ws",
                Protocol::VmessWs => "vmess-ws",
                Protocol::Shadowsocks => "shadowsocks",
                Protocol::Trojan => "trojan",
                Protocol::Tuic => "tuic",
                Protocol::Anytls => "anytls",
                Protocol::Hysteria2 => "hysteria2",
                Protocol::Unknown => "unknown",
            }
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundNode {
    pub tag: String,
    pub protocol: Protocol,
    pub listen_port: u16,
    pub user_count: usize,
}

#[derive(Debug, Clone)]
pub struct AddNodeRequest {
    pub tag: String,
    pub protocol: Protocol,
    pub listen_port: u16,
    pub server_name: Option<String>,
    pub path: Option<String>,
    pub port_reuse: bool,
}
