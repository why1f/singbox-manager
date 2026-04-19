use clap::{Args, Subcommand};

#[derive(Args, Debug)]
pub struct KernelArgs { #[command(subcommand)] pub command: KernelCommands }

#[derive(Subcommand, Debug)]
pub enum KernelCommands {
    /// 显示 sing-box 内核状态
    Status,
    /// 调用官方脚本安装最新 sing-box (不含 v2ray_api)
    Install,
    /// 从本仓库 release 安装自编译 sing-box (含 v2ray_api)
    InstallV2rayApi,
    /// 停止并卸载 sing-box（保留 /etc/sing-box 配置）
    Uninstall,
    /// 启动
    Start,
    /// 停止
    Stop,
    /// 重启
    Restart,
    /// 设置开机自启
    Enable,
    /// 关闭开机自启
    Disable,
}
