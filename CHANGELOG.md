# CHANGELOG

## v0.4.5

### Fixed
- **CLI 用户操作不再对不存在用户误报成功**：`sb del`、`sb reset`、`sb pkg` 现在会先校验用户是否存在，不存在时直接返回明确错误，避免脚本化运维时出现“看起来成功但实际没生效”的假阳性。

## v0.4.4

### Fixed
- **删除用户/节点后的脏引用清理**：删除节点后会同步清理用户 `allowed_nodes` 中的残留 tag；删除用户后重新同步配置时，会移除各协议 inbound `users` 里的旧账号条目，避免界面和 `config.json` 继续显示已删除对象。
- **用户页手动重置流量增加确认**：`r` 不再直接执行重置，先弹确认框，避免和 `R` 刷新误触。
- **用户表单到期日校验补齐**：TUI 添加/编辑用户时，本地就会校验 `YYYY-MM-DD` 格式，减少错误输入落库。
- **订阅节点名不再自动追加用户名**：分享链接和 Clash/Mihomo 节点名改为仅使用节点 tag，避免名称被自动拼成 `节点-用户名`。
- **CLI 授权节点时增加存在性校验**：`sb grant` 现在会先检查节点 tag 是否存在，避免把无效分配写入数据库。

## v0.4.3

### Fixed
- **用户页流量进度条对齐修复**：`配额/进度` 列改为固定宽度排版，不同配额值下进度条起始位置保持一致。

## v0.4.2

### Added
- **新增 `sb doctor` 自检命令**：统一检查配置文件、数据库、`sing-box check`、`v2ray_api`、gRPC、证书文件、订阅配置和 nginx 状态，部署后排障更直接。
- **CLI 节点管理补齐编辑/删除**：`sb node edit` / `sb node del` 与 TUI 的节点操作基本对齐，脚本化运维不再只能进 TUI。

### Fixed
- **订阅生成失败不再静默返回空内容**：`/sub/<token>` 在节点地址解析或订阅导出失败时改为明确返回错误，避免客户端把服务端异常误判成“没有节点”。
- **节点地址不再静默回退到 `127.0.0.1`**：优先使用 `subscription.public_base` 主机名或反代 `Host`，公网 IP 探测失败时直接报错，避免导出不可用节点。
- **`subscription-userinfo` 口径对齐倍率计费**：订阅响应头中的上传/下载统计改为按 `traffic_multiplier` 后的有效流量输出。
- **用户参数校验补齐**：新增配额、重置日、到期日、流量倍率的合法性校验，避免脏数据写入数据库。
- **月重置日支持到 31 号**：之前 `29/30/31` 会被当成不重置，现改为按当月最后一天自动收敛。

### Changed
- **release workflow 增加版本元数据校验**：发 tag 时会先校验 `Cargo.toml`、`CHANGELOG.md` 与 tag 一致，减少发版漂移。
- **release workflow 固定 `cross` 版本**：避免直接从 Git 仓库 HEAD 安装带来的构建不稳定。

## v0.4.1

### Fixed
- **流量告警倍率口径统一**：配额告警改为与总量展示、超额封禁一致，按 `traffic_multiplier` 后的有效用量计算，避免双向计费场景下告警偏晚。
- **备份恢复重启错误服务名**：恢复完成后改为重启正确的 `sb-manager` 服务，而非不存在的 `singbox-manager`。

### Changed
- **订阅服务公网 IP 查询增加缓存**：`/sub` 不再每次请求都实时探测公网 IP，减少外部依赖与响应延迟。
- **备份恢复增加路径白名单校验**：恢复前先校验 tar 包内路径，只允许恢复 `manager / certs / config.json / nginx conf`。
- **安装脚本补 shell fallback**：`install-release.sh` 在 `python3` 不可用时，仍会用 shell 方式更新 `config.toml` 里的路径字段。

## v0.4.0

### Added
- **完整的一键式系统备份与恢复功能**：支持备份面板配置、数据库、证书及订阅文件，支持最多保存5个历史备份。在 TUI 仪表盘按 `b` 备份，按 `r` 恢复。
- **全新的“绿色化”目录结构**：所有相关文件均统一存放在 `/etc/sing-box/` 目录下（包括主程序、内核、数据库、配置、证书等），实现了真正的一键清理。

### Changed
- **[BREAKING]** 面板配置文件名由 `config.toml` 移至 `/etc/sing-box/manager/config.toml`。
- **[BREAKING]** 默认生成的 `config.json` 及 `manager.db` 路径也已随之变更为统一目录。老用户升级需清理重装或手动迁移。
- **[BREAKING]** 废弃官方脚本安装代理内核，现由面板独立完成纯净二进制的下载安装（不包含 `.deb` 残留），完全接管并净化内核生态。
## v0.3.13

### Added

- **用户流量倍率 (Traffic Multiplier)**：创建或编辑用户时可指定流量倍率（如双向计费的 2.0 或单向的 1.0），默认 2.0。面板所有流量显示、超额封禁判断、订阅流量展示将全部接管倍率计算。
- **用户列表 TUI 更新**：新增 `计费` 列，可视化显示“双向”、“单向”或具体的倍率。

## v0.3.9

### Fixed

- **流量重置后虚报峰值**：`reset_usage` / `reset_usage_manual` 不再清零 `last_live_up/down`。  
  gRPC 流量计数器是自 sing-box 启动以来的累计值，清零会导致重置后下次同步 `calc_delta(gRPC累计, 0)` 把全部历史累计量重新计入 `used_bytes`，造成流量数值暴涨。保留旧累计值后，增量计算只统计重置后的新增量

## v0.3.8

### Fixed

- **SS 节点 Clash 订阅消失**：`clash_ss` 读 `user["uuid"]` 而非 `user["password"]`（SS 用户条目无 uuid 字段），导致 Clash/Mihomo 订阅里 SS 节点始终为空
- **手动重置污染月度自动重置**：新增 `reset_usage_manual`，手动重置不写 `last_reset_ym`，避免同月手动重置后当月定期重置被跳过
- **月重置后超额用户永久禁用**：月重置执行后同时调用 `set_enabled(true)` 并 `continue`，确保超额被禁的用户在重置日自动解封
- **TUI 模式 gRPC 失败时自动控制不执行**：TUI 后台重连循环的 `Err` 分支补调 `apply_automatic_controls`，与 daemon 模式行为对齐
- **vless-ws 订阅链接 TLS 字段错误**：`vless_ws` 不再无条件写 `security=tls`，改为按 `tls.enabled` 动态生成 `tls/none`
- **Clash 订阅缺 anytls 节点**：`generate_clash_yaml` 新增 `"anytls"` match 分支及 `clash_anytls` 函数
- **profile-web-page-url 响应头错误**：改为 `{public_base}/sub/{token}` 完整路径，客户端点击跳转不再 404
- **IPv6 服务器订阅链接格式非法**：`get_server_ip` 自动检测并为 IPv6 地址添加 `[...]` 包裹
- **vmess 协议检测误判**：按 `transport.type` 区分 ws/tcp，非 ws 的 vmess 不再误报为 `vmess-ws`

## v0.3.7

### Fixed

- 内核页 [5] 按数字 `6` 切不到 nginx 页（`handle_kernel_key` 漏了 `'6'` 分支）。Tab 一直能用是因为走的是 `s.next_page()`，不走 match

## v0.3.6

### Changed

- `[C]` 编辑 config.json 的编辑器回落链从 `${EDITOR:-nano} || vi` 改为 `${EDITOR:-vim} || nano || vi` —— 裸机没设 `$EDITOR` 直接上 vim

## v0.3.5

### Added

- **TUI 挂起式外部命令机制**：通用基础设施，按键只写 `AppState.pending_cmd`，主 loop 下一轮挂起 TUI (LeaveAlternateScreen + disable_raw_mode) → 让子进程继承 TTY → Enter 返回 → 恢复 TUI。同步阻塞当前 tokio worker，后台任务仍在另一个 worker 跑。相比之前 v0.3.5 被 revert 的 tokio pipe + 流式日志方案代码量少三分之一且无 root 检查 / cfg gate
- **Dashboard `[U]` 一键升级**：`curl -fsSL .../install-release.sh | sudo bash`，sudo 问密码 TTY 已让出可正常输入。脚本自带版本比对，已最新会跳过
- **Nodes `[C]` 编辑 sing-box config.json**：`$EDITOR` → 回落 `nano` → `vi`，退出后自动 `sing-box check`，通过则若运行中即 `reload`，失败在状态栏提示不 reload
- **Logs `[f]` 实时 sing-box 日志**：`journalctl -u sing-box -f -n 50`，`Ctrl-C` 回 TUI

### Notes

- 之前 v0.3.5（tokio pipe 方案）已 revert + tag 删除，本 v0.3.5 是重新实现的干净版
- 三个外部命令场景共用一套挂起机制，后续新增每个 ~10 行

## v0.3.4

### Added

- 用户编辑表单到期字段支持填 `-` 将已有到期日**清为永久**（之前留空的语义是"不改"，没法清，只能 `sb pkg <user> -e ""`）

## v0.3.3

### Fixed

- **节点列表端口复用列选中后看不清**：旧配色用 DarkGray 做 "关 / 不支持"，跟选中行的 DarkGray 背景重叠成糊一片。改为不染色继承行样式（选中时白字+灰底，清晰可读），"开" 保留绿色
- 用户添加/编辑表单的"到期"字段标签补上示例 `例: 2026-12-31`，避免用户不知道格式

## v0.3.2

### Added

- **节点列表表格新增"端口复用"列**：支持的协议（reality/trojan/anytls）显示 `● 开` / `○ 关`，不支持的（hy2/tuic/ss/*-ws）显示 `─ 不支持`
- **添加节点表单同步加端口复用开关**（仅 reality/trojan/anytls 协议下显示）：
  - 建节点时直接勾上，不用建完再编辑
  - CLI `sb add-node` 新增 `--port-reuse` 标志
- 添加表单底部动态提示"端口复用开启：listen→127.0.0.1，订阅端口写 443；需手动配 nginx stream SNI 分流"

## v0.3.1

### Added

- **节点端口复用开关**（仅 vless-reality / trojan / anytls）：TUI 编辑节点时多一个 `端口复用` 开关，开启后
  - sing-box inbound `listen` 自动改写为 `127.0.0.1`（由 nginx stream 回源）
  - 订阅 URL 的 port 固定写 443（不跟 `listen_port`）
  - 状态存 `NodeMeta.port_reuse`，重启保留
  - 同时提示用户需手动配 nginx stream `ssl_preread` SNI 分流（README 新增模板段）
- 节点页选中栏新增端口复用状态显示：`内部 4433 · 对外 443 (端口复用)`

### Notes

- 不支持端口复用的协议（hy2/tuic UDP、ss/vless-ws/vmess-ws 无 SNI）编辑表单不显示该字段
- sing-box ≥1.11 inbound 已不支持 `proxy_protocol`，故我们不走 PROXY header 方案；代价是日志里 `remote_addr` 总是 127.0.0.1，鉴权/统计不受影响

## v0.3.0

### Added

- **浏览器打开订阅 URL = 流量统计 HTML 页面**：同一条 `/sub/<token>` 按 User-Agent 分流：
  - `Mozilla/*`（浏览器）→ 深色主题 HTML 页：流量进度条（按百分比分段 绿/黄/红）、上下行分拆、重置日 / 到期日倒计时、整体订阅链接（sing-box + mihomo 双行，复制按钮）、单节点链接列表（每条配可展开的 QR SVG）
  - `clash-meta` / `mihomo` / `stash` / `clashx` → mihomo yaml
  - 其他（sing-box / v2rayN / 未识别客户端）→ base64 sing-box
- **`?type=` 显式覆盖**：`?type=stats|clash|mihomo|yaml|sing-box|base64` 优先于 UA 分流；浏览器里加 `?type=clash` 可直接查看 yaml 源码
- **Token 撤销**：
  - CLI `sb token revoke <user>` 把 token 置空，`/sub/` 立即 404；再 `sb token regen` 可恢复
  - TUI 用户页 `[T]` 打开 token 管理弹窗：`[g]` 重新生成 / `[v]` 撤销
- 新增依赖 `qrcode 0.14`（`default-features = false, features = ["svg"]`，仅 SVG 渲染，几十 KB）

### Changed

- 用户页操作栏：`[a]添加 [E]编辑 [d]删除 [t]启/禁 [r]重置 [T]token [u]复制URL [s]打印 [n]分配节点 [R]刷新`
- `ShareLink` 结构体新增 `tag` 字段，用于 HTML 页面按节点分组展示

## v0.2.6

### Changed

- nginx 反代模板的证书默认路径改为 `/etc/nginx/ssl/{host}.crt` / `.key`（平铺命名），更贴近手签 / 宝塔 / 常见站长习惯；acme.sh 用户改成 `/etc/nginx/certs/{host}/fullchain.pem` 之类的老路径即可

## v0.2.5

### Fixed

- **nginx 反代 conf 模板 `nginx -t` 失败**：`location ~ ^/sub/[A-Za-z0-9_-]{16,64}$` 正则没加引号，nginx 把第一个 `{` 当成 location block 起始，于是 `16,64}$ {` 被当成下一条 directive → `unknown directive "16,64}$"`。改为 `location ~ "^/sub/[A-Za-z0-9_-]{16,64}$"` 用双引号包裹正则。已经 `[g]` 过坏 conf 的用户，再跑一次 `[g]` / `sb nginx gen-conf` 覆盖即可

## v0.2.4

### Fixed

- **hysteria2 inbound 莫名其妙的 `server_name: bing.com`**：之前所有 TLS 协议共用了同一套默认 SNI，hy2 被硬塞了 `tls.server_name`；对照参考脚本 `20_protocol.sh` 与 sing-box 官方 hy2 示例，hy2 inbound 本就不该带 server_name，现在不再写入（自签证书 CN 改用 tag 本身）
- **自签证书协议订阅链接缺 `insecure=1`**：`insecure_flag()` 旧判定 `certificate_path 为空才算自签`，我们走自签时 path 永远非空，flag 永远返回 false。改为判定路径是否在托管目录 `/etc/sing-box-manager/certs/` 下。影响面：trojan / tuic / anytls / hysteria2 客户端 TLS 校验现在能正确通过

### Changed

- **节点 add/edit 表单按协议显示字段**，拒绝"无中生有"：
  - reality/trojan/tuic/anytls：显示 `server_name`
  - vless-ws/vmess-ws：显示 `path`
  - hysteria2/shadowsocks：仅 tag+协议+端口
  - 未激活字段的值不会回传到 `AddNodeRequest`，即便用户通过 CLI `--server-name` 传了也会在 hy2 构建时被忽略
- `edit_node` 只在 inbound 的 `tls` 已有 `server_name` 键时才更新，杜绝给不该有的协议塞字段

### Docs

- README 补齐 `sb grant/revoke/grant-all/allowed`、`sb token`、`sb nginx`、TUI 第 6 页 nginx、节点编辑按 `[E]`、`[subscription]` 配置段

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
