use clap::{Args, Subcommand};

#[derive(Args, Debug)]
pub struct BindArgs {
    #[command(subcommand)]
    pub command: BindCommands,
}

/// Telegram 绑定码管理。绑定码是一次性的：用户 `/bind <码>` 成功后立即作废，
/// 需要再绑必须先 `sb bind regen`。
#[derive(Subcommand, Debug)]
pub enum BindCommands {
    /// 显示用户的绑定状态与当前绑定码
    Show { name: String },
    /// 重新生成绑定码（旧码立即失效，不影响已建立的绑定）
    Regen { name: String },
    /// 解除该用户已有的 TG 绑定
    Unbind { name: String },
    /// 列出所有用户的绑定状态
    List,
}
