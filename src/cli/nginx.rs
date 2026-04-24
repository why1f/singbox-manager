use clap::{Args, Subcommand};

#[derive(Args, Debug)]
pub struct NginxArgs {
    #[command(subcommand)]
    pub command: NginxCommands,
}

#[derive(Subcommand, Debug)]
pub enum NginxCommands {
    /// 状态
    Status,
    /// 用包管理器安装 nginx
    Install,
    /// 启动 / 停止 / 重启 / 重载
    Start,
    Stop,
    Restart,
    Reload,
    /// 开机自启 / 关闭自启
    Enable,
    Disable,
    /// 生成 sb-manager 反代配置到 [subscription].nginx_conf
    GenConf,
    /// nginx -t 语法检查
    Test,
}
