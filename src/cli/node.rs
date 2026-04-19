use clap::{Args, Subcommand};
use crate::model::node::Protocol;

#[derive(Args, Debug)]
pub struct NodeArgs { #[command(subcommand)] pub command: NodeCommands }

#[derive(Args, Debug)]
pub struct AddNodeArgs {
    pub tag: String,
    #[arg(short, long)] pub protocol: String,
    #[arg(short, long)] pub port: u16,
    #[arg(long)] pub server_name: Option<String>,
    #[arg(long)] pub path: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum NodeCommands {
    List,
    Export { name: String },
    Add(AddNodeArgs),
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
