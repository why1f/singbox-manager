use crate::model::node::Protocol;
use clap::{Args, Subcommand};

#[derive(Args, Debug)]
pub struct NodeArgs {
    #[command(subcommand)]
    pub command: NodeCommands,
}

#[derive(Args, Debug)]
pub struct AddNodeArgs {
    pub tag: String,
    #[arg(short, long)]
    pub protocol: String,
    #[arg(short, long)]
    pub port: u16,
    #[arg(long)]
    pub server_name: Option<String>,
    #[arg(long)]
    pub path: Option<String>,
    /// 端口复用：inbound listen 改 127.0.0.1，订阅 port 固定 443（仅 reality/trojan/anytls 有效）
    #[arg(long, default_value_t = false)]
    pub port_reuse: bool,
    /// 订阅导出时使用 IPv6 地址
    #[arg(long, default_value_t = false)]
    pub ipv6: bool,
    /// 中转机 IP/域名：设了之后订阅里该节点的地址换成它（转发需中转机自己实现）
    #[arg(long)]
    pub relay_host: Option<String>,
    /// 中转机端口，留空则沿用节点端口
    #[arg(long)]
    pub relay_port: Option<u16>,
}

#[derive(Args, Debug)]
pub struct EditNodeArgs {
    pub tag: String,
    #[arg(long)]
    pub port: Option<u16>,
    #[arg(long)]
    pub server_name: Option<String>,
    #[arg(long)]
    pub path: Option<String>,
    #[arg(long)]
    pub port_reuse: Option<bool>,
    #[arg(long)]
    pub ipv6: Option<bool>,
    /// 中转机 IP/域名；传空串 `--relay-host ""` 即关闭中转
    #[arg(long)]
    pub relay_host: Option<String>,
    /// 中转机端口，留空则沿用节点端口
    #[arg(long)]
    pub relay_port: Option<u16>,
}

#[derive(Subcommand, Debug)]
pub enum NodeCommands {
    List,
    Export { name: String },
    Add(AddNodeArgs),
    Edit(EditNodeArgs),
    Del { tag: String },
}

impl TryFrom<&str> for Protocol {
    type Error = anyhow::Error;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "vless-reality" => Ok(Protocol::VlessReality),
            "vless-ws" => Ok(Protocol::VlessWs),
            "vmess-ws" => Ok(Protocol::VmessWs),
            "trojan" => Ok(Protocol::Trojan),
            "shadowsocks" => Ok(Protocol::Shadowsocks),
            "hysteria2" => Ok(Protocol::Hysteria2),
            "tuic" => Ok(Protocol::Tuic),
            "anytls" => Ok(Protocol::Anytls),
            _ => Err(anyhow::anyhow!("不支持的协议: {}", value)),
        }
    }
}
