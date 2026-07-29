use clap::{Args, Subcommand};

#[derive(Args, Debug)]
pub struct OutboundArgs {
    #[command(subcommand)]
    pub command: Option<OutboundCommands>,
}

#[derive(Subcommand, Debug)]
pub enum OutboundCommands {
    /// 显示当前出站地址族策略（不带子命令时的默认行为）
    Show,
    /// 设置出站地址族策略
    ///
    /// MODE 取 auto / prefer4 / prefer6 / v4only / v6only，
    /// 也接受 sing-box 原值 prefer_ipv4 / ipv4_only 等写法。
    Set {
        /// auto | prefer4 | prefer6 | v4only | v6only
        mode: String,
    },
}
