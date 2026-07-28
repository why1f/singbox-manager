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
    pub ipv6: bool,
    /// 中转（前置转发）设置。只改订阅里导出的地址，不动 sing-box inbound。
    pub relay: RelaySetting,
}

/// 中转设置：订阅导出时把节点地址换成中转机的地址。
///
/// 典型用法是落地机在墙外、另有一台线路更好的中转机做 TCP/UDP 转发，
/// 客户端连中转机、由它转到本机的 `listen_port`。
/// **转发本身由中转机自己实现**，本工具只负责让订阅指向它。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RelaySetting {
    /// 中转机地址（IP 或域名）。空串表示不启用中转。
    pub host: String,
    /// 中转机端口。留空则沿用节点自身对外端口。
    pub port: Option<u16>,
}

impl RelaySetting {
    pub fn is_enabled(&self) -> bool {
        !self.host.trim().is_empty()
    }
}

/// 编辑节点请求。字段为 `None` 表示"保持原样"。
///
/// 用结构体而不是长参数列表：edit_node 的可改项已经到了 8 个，
/// 再按位置传参极容易把 server_name 和 path 之类的同型参数搞反。
#[derive(Debug, Clone, Default)]
pub struct EditNodeRequest {
    pub tag: String,
    pub listen_port: Option<u16>,
    pub server_name: Option<String>,
    pub path: Option<String>,
    pub port_reuse: Option<bool>,
    pub ipv6: Option<bool>,
    /// `None` = 不改中转设置；`Some` = 整组覆盖（`host` 为空即关闭中转）。
    pub relay: Option<RelaySetting>,
}
