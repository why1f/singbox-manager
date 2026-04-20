pub mod kernel;
pub mod nginx;
pub mod node;
pub mod token;
pub mod user;
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name="sb", about="sing-box 管理工具", version)]
pub struct Cli {
    #[arg(short, long, global=true)] pub config: Option<String>,
    #[command(subcommand)]          pub command: Option<Commands>,
}
#[derive(Subcommand, Debug)]
pub enum Commands {
    #[command(about="列出用户")]         Users,
    #[command(about="添加用户")]         Add(user::AddUserArgs),
    #[command(about="删除用户")]         Del { name: String },
    #[command(about="启用用户")]         On { name: String },
    #[command(about="禁用用户")]         Off { name: String },
    #[command(about="重置用户流量")]     Reset { name: String },
    #[command(about="查看用户详情")]     Info { name: String },
    #[command(about="导出用户订阅")]     Sub { name: String },
    #[command(about="调整用户套餐")]     Pkg(user::PackageArgs),
    #[command(about="授权用户可用某节点")] Grant { name: String, tag: String },
    #[command(about="撤销用户可用某节点")] Revoke { name: String, tag: String },
    #[command(about="恢复用户所有节点可用")] GrantAll { name: String },
    #[command(about="显示用户当前允许的节点")] Allowed { name: String },
    #[command(about="列出节点")]         Nodes,
    #[command(about="添加节点")]         AddNode(node::AddNodeArgs),
    #[command(about="导出节点订阅")]     Export { name: String },
    #[command(about="检查 sing-box 配置")] Check,
    #[command(about="启动 sing-box")]    Start,
    #[command(about="停止 sing-box")]    Stop,
    #[command(about="重载 sing-box")]    Reload,
    #[command(about="查看服务状态")]     Status,
    #[command(about="后台守护模式")]     Daemon,
    #[command(about="启动 TUI（默认）")] Tui,
    #[command(about="sing-box 内核管理")] Kernel(kernel::KernelArgs),
    #[command(about="订阅 token 管理")]   Token(token::TokenArgs),
    #[command(about="nginx 管理")]        Nginx(nginx::NginxArgs),
    #[command(about="兼容旧用户命令")]   User(user::UserArgs),
    #[command(about="兼容旧节点命令")]   Node(node::NodeArgs),
}
