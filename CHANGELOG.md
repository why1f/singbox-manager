# CHANGELOG

## v0.1.3

### Fixed

- **gRPC 流量统计完全不通**：proto 包名从 `experimental.v2rayapi` 改为 `v2ray.core.app.stats.command`。sing-box 在 `init()` 里把 ServiceDesc.ServiceName 覆写成 v2ray 兼容路径，所以无论 build tag 对不对，旧 proto 都永远返回 Unimplemented
- 添加/删除节点时 `sing-box check` 失败会让 TUI 显示"No such file or directory"但节点其实已写入 config.json，需要退出重进 —— 现在不管 check/reload 结果都发 NodesRefreshed
- `install-release.sh` 丢失 `install -m 0755` 命令，下载解压但没拷到 `/usr/local/bin/sb`
- `build-singbox` workflow sha256 文件命名改为 `<asset>.sha256`，与 Rust 内核安装侧期望一致
- 嵌套 `if let` 触发 `clippy::collapsible_match` 致 CI 失败

### Added

- **用户-节点分配** (schema v2)：新增 `users.allowed_nodes` JSON 字段
  - `sb grant/revoke/grant-all/allowed <user> <tag>` CLI
  - 用户页按 `[n]` 打开节点选择弹窗（Space 勾选，`a` 切换全部模式）
  - `sync_users` 按 `can_use_node(tag)` 过滤
- **reality 节点密钥自动生成**：`add-node vless-reality` 时自动调用 `sing-box generate reality-keypair`，写入 private_key + short_id + handshake；public_key 作为非标字段保留供订阅生成读取，TUI 状态条回显给用户
- **仪表盘 Top5 用量展示**：用户摘要块增加按总流量排序的前 5 名
- **状态条 5s 自动清除**
- **内核页操作后轮询 3×500ms 刷新状态**（避开 systemctl 返回 vs pgrep 可见性竞态）

### Changed

- `install_v2rayapi` 成功后自动 `systemctl enable + restart`，无需手动启动
- `build-singbox` workflow 读 upstream `release/DEFAULT_BUILD_TAGS` + `release/LDFLAGS` 与官方 release 一致（含 naive 等）；追加 `with_v2ray_api` + `with_purego`，体积对齐 ~55MB
- `build-singbox` 增加 `force` 开关，可重建同 tag 的 release

## v0.1.2

### Added

- **TUI 内核页**（第 5 页）：一键装/卸载/启停/重启/自启 sing-box
  - `[i]` 装官方版（`sing-box.app` 脚本）
  - `[v]` 装 v2ray_api 版（从本仓库 release 下载，带流量统计 gRPC）
- **TUI 表单**：用户页 `[a]` 添加 `[d]` 删除；节点页 `[a]` 添加 `[d]` 删除
  - Modal 弹窗 + 删除确认，Tab/方向键切换字段，←/→ 选协议
- **自动构建 sing-box**：`.github/workflows/build-singbox.yml` 每日检查 upstream 最新 tag，带 `with_v2ray_api` 构建 linux-amd64 + linux-arm64
- CLI 新增 `sb kernel <status|install|install-v2ray-api|uninstall|start|stop|restart|enable|disable>`
- `config.toml` 新增 `[kernel] update_repo` 指定从哪个 fork 拉 v2ray_api 版

### Changed

- `install.sh` / `install-release.sh` 不再强装 sing-box，缺失时引导进 TUI 内核页
- 新增 `/etc/profile.d/sb-manager.sh` 自动清理可能的 stale `sb` / `sing-box` alias
- `install.sh` 支持 apt/dnf/yum/pacman/apk，去掉 libssl-dev 依赖（reqwest 走 rustls）

## v0.1.1

### Fixed

- daemon 无 gRPC 重连，现改为指数退避重试（1s→60s 上限）
- 配额告警每个同步周期都重复发送，现按阈值档位去重（80/95/100）
- DB 迁移 `ALTER TABLE ... .ok()` 吞错误，改用 `PRAGMA user_version` 显式版本
- TUI `handle_key` 在事件循环里阻塞同步 IO（导出订阅/刷新会卡 UI），改 `tokio::spawn` + UiEvent 回执通道
- `get_server_ip` 无超时，网络异常可挂 90s+，加 3s timeout
- 订阅链接 `allowInsecure=1` 硬编码，改根据 inbound TLS 类型自动判断
- 订阅 fragment 对 hysteria2/tuic/anytls 硬编码丢信息，改 `{tag}-{name}` 统一

### Changed

- 依赖瘦身：`reqwest` 切 rustls，`tokio`/`sqlx`/`tonic`/`chrono` feature 精简
- `user_service::update_package` 三次 UPDATE 合并为 COALESCE 单条
- `set_user_enabled` / `toggle_user` 改单事务原子操作
- 历史清理 `timestamp() % 3600` 改独立 1h interval
- systemd unit 增加硬化项（NoNewPrivileges / ProtectSystem=strict / ReadWritePaths）

## v0.1.0

### Added

- Rust 工程化项目结构，SQLite 用户与流量历史存储
- 用户凭据字段支持（uuid / password）
- 短命令 CLI：`sb users/add/del/on/off/reset/info/sub/pkg/add-traffic`
- 服务管理命令：`sb check/start/stop/reload/status`
- daemon 后台模式，gRPC 流量同步
- 自动控制：到期禁用 / 超额禁用 / 月重置
- 节点解析与节点列表
- 最小节点新增闭环：`sb add-node`
- 订阅导出与 Base64 订阅
- TUI 主面板：高频操作（启禁、重置、刷新、配置校验、订阅输出到日志页）
- Linux 安装脚本与 systemd service

### Notes

- 定位：轻量 CLI + TUI，不做 Web 面板
- 适合单机 / 小规模 / 自用 / 内部工具场景
