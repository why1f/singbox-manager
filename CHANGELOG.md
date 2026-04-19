# CHANGELOG

## v0.1.0-beta

### Added

- Rust 工程化项目结构
- SQLite 用户与流量历史存储
- 用户凭据字段支持（uuid / password）
- 短命令 CLI：`sb users/add/del/on/off/reset/info/sub/pkg/add-traffic`
- 服务管理命令：`sb check/start/stop/reload/status`
- daemon 后台模式
- gRPC 流量同步
- 自动控制（到期禁用 / 超额禁用 / 月重置）
- 节点解析与节点列表
- 最小节点新增闭环：`sb add-node`
- 订阅导出与 Base64 订阅
- TUI 主面板
- TUI 高频真实操作：启禁、重置、刷新、配置校验、订阅输出到日志页
- Linux 安装脚本与 systemd service

### Changed

- 项目定位明确收敛为轻量 CLI + TUI 工具
- 不再朝 Web 面板方向扩展
- 用户变更已联动 sing-box 配置与 reload

### Notes

- 当前版本适合单机 / 小规模 / 自用 / 内部工具场景
- 当前仍建议作为 0.x 版本持续迭代
