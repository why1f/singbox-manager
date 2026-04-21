use clap::{Args, Subcommand};

#[derive(Args, Debug)]
pub struct TokenArgs { #[command(subcommand)] pub command: TokenCommands }

#[derive(Subcommand, Debug)]
pub enum TokenCommands {
    /// 打印用户的订阅 URL 与 token
    Show { name: String },
    /// 重新生成 token（旧 URL 立即失效）
    Regen { name: String },
    /// 撤销 token（关闭订阅，/sub/ 返回 404；再 regen 可恢复）
    Revoke { name: String },
}
