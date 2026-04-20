# singbox-manager

面向 sing-box 的**轻量 CLI + TUI 管理工具**（Rust）。不做 Web，不做平台，只干四件事：节点搭建 / 用户管理 / 流量统计 / 订阅导出。

- **单机/小规模** 一台机一人管，别装一堆东西
- **静态 musl 二进制** ~10 MB，不依赖 glibc 版本
- **内核管理内置** 在 TUI 里一键装官方 sing-box 或带 `with_v2ray_api` 的自编译版

---

## 安装（推荐：预编译二进制）

> 适用：Linux amd64 / arm64（任意发行版），只需 `curl` + `tar` + `systemd`，无需 Rust/gcc。

```bash
# 以 root 执行
curl -fL https://raw.githubusercontent.com/why1f/singbox-manager/master/install-release.sh -o install-release.sh
sudo REPO=why1f/singbox-manager bash install-release.sh
sudo systemctl start sb-manager
```

脚本会：
1. 下载最新 `sb` 二进制到 `/usr/local/bin/sb`
2. 写 `/etc/sing-box-manager/config.toml`、`/etc/systemd/system/sb-manager.service`
3. 建软链 `/usr/bin/sb` + 清 `/etc/profile.d/sb-manager.sh` 里的 stale alias
4. `systemctl enable sb-manager`

指定版本：

```bash
sudo REPO=why1f/singbox-manager VERSION=v0.1.2 bash install-release.sh
```

**装完直接敲 `sb` 进 TUI**。如果提示 command not found：
```bash
unalias sb sing-box 2>/dev/null; hash -r
# 或重登 shell（/etc/profile.d/sb-manager.sh 会自动清理）
```

---

## 安装（备选：源码编译）

需要 root + Ubuntu/Debian/RHEL 系发行版：

```bash
git clone https://github.com/why1f/singbox-manager.git
cd singbox-manager
sudo bash install.sh
sudo systemctl start sb-manager
```

自动装 Rust toolchain、编译、部署。大约 5-10 分钟。

---

## 首次使用：装 sing-box 内核

```bash
sb      # 进 TUI
```

按 `5` 切到 **内核** 页，选其一：

| 按键 | 操作 |
|---|---|
| `i` | 装**官方版** sing-box（走 `sing-box.app` 脚本，不含 `with_v2ray_api`，**流量统计不可用**） |
| `v` | 装 **v2ray_api 版**（从本仓库 release 下载，**推荐**，带流量统计） |
| `s` / `S` / `x` | 启动 / 停止 / 重启 |
| `e` / `d` | 开启 / 关闭开机自启 |
| `u` | 卸载（保留 `/etc/sing-box` 配置） |
| `R` | 刷新状态 |

**v2ray_api 版**是本仓库 GitHub Actions 每天基于 upstream [SagerNet/sing-box](https://github.com/SagerNet/sing-box) 自动构建的，tag 形如 `singbox-vX.Y.Z`。与官方 release 的唯一区别是启用了 `with_v2ray_api` build tag + `with_purego`（让 naive 免 CGO）。

> 小知识：sing-box 官方 release 为了轻量默认**不启用** `with_v2ray_api`，而这是流量统计 gRPC 接口所必需的。所以要么用本项目构建的版本，要么自己从源码编。

---

## 常用命令（CLI）

TUI 是主入口，CLI 做脚本集成用。

### 用户

```bash
sb users                                        # 列表
sb add alice -q 100 -r 1 -e 2026-12-31          # 100GB/月，每月 1 号重置，年底到期
sb info alice                                   # 详情
sb on alice / sb off alice                      # 启用 / 禁用
sb reset alice                                  # 清零流量
sb pkg alice -q 200                             # 改配额（只动这一项）
sb sub alice                                    # 打印该用户的订阅链接
sb del alice
```

### 用户可用节点授权

默认新建用户可用所有节点。收敛范围：

```bash
sb revoke alice vless1                          # 从 alice 撤销 vless1
sb grant  alice vless2                          # 授权 alice 用 vless2
sb grant-all alice                              # 恢复全部节点可用
sb allowed alice                                # 查看当前允许列表
```

### 订阅 token / HTTP 服务

daemon/TUI 会起一个本地订阅 HTTP（默认 `127.0.0.1:18081`），给每个用户分配一个 token，前挂 nginx 反代即可对外分发订阅。

```bash
sb token show alice                             # 打印订阅 URL + token
sb token regen alice                            # 轮换 token（旧 URL 立即失效）
```

### 节点

```bash
sb nodes
sb add-node vless1 -p vless-reality --port 443 --server-name www.apple.com
sb add-node vless2 -p vless-ws      --port 8443 --path /vless
sb add-node hy1    -p hysteria2     --port 8443       # 无需 server_name / path
sb export alice                                 # Base64 订阅 + 明文链接
```

协议：`vless-reality` / `vless-ws` / `vmess-ws` / `trojan` / `shadowsocks` / `hysteria2` / `tuic` / `anytls`

各协议需要的字段不同：
- `vless-reality` / `trojan` / `tuic` / `anytls`：`--server-name`（SNI / 自签证书 CN）
- `vless-ws` / `vmess-ws`：`--path`
- `hysteria2` / `shadowsocks`：只要 `--port`（多填会被忽略）

### sing-box 内核

```bash
sb kernel status
sb kernel install                   # 官方版
sb kernel install-v2ray-api         # v2ray_api 版（推荐）
sb kernel start / stop / restart
sb kernel enable / disable          # 开机自启
sb kernel uninstall
```

### nginx 反代

```bash
sb nginx install                    # 用发行版包管理器装
sb nginx gen-conf                   # 按 config.toml 生成反代 conf 到 [subscription].nginx_conf
sb nginx test                       # nginx -t 语法检查
sb nginx start / stop / restart / reload
sb nginx enable / disable
sb nginx status
```

### 服务状态

```bash
sb status                           # sing-box + gRPC + 配置路径
sb check                            # 校验 sing-box 配置
sb reload                           # 重载 sing-box
```

---

## TUI 操作速查

```
  [1-6]       切换页：仪表盘 / 用户 / 节点 / 日志 / 内核 / nginx
  [Tab]       下一页
  [q] / Ctrl+C 退出
  [Esc]       清当前状态提示
  [↑↓/jk]     在列表里选中
  [R]         刷新
  [Enter]     弹窗里确认提交
```

**用户页**：`[a]` 添加 `[E]` 编辑 `[d]` 删除 `[t]` 启禁 `[r]` 重置流量 `[s]` 导出订阅 `[n]` 分配可用节点

**节点页**：`[a]` 添加 `[E]` 编辑 `[d]` 删除（弹窗表单按当前协议显示字段：reality/trojan/tuic/anytls 多一行 `server_name`；vless-ws/vmess-ws 多一行 `path`；hysteria2/shadowsocks 只要端口）

**内核页**：`[i]` 装官方 `[v]` 装 v2ray_api 版 `[u]` 卸载 `[s/S/x]` 启/停/重启 `[e/d]` 开/关自启

**nginx 页**：`[i]` 装 `[g]` 生成反代 conf `[t]` 语法检查 `[s/S/x]` 启/停/重启 `[r]` reload `[e/d]` 开/关自启

添加节点后会自动 `sing-box check -c ... && systemctl reload sing-box`，校验不通过不会覆盖。

---

## 升级

```bash
sudo REPO=why1f/singbox-manager bash install-release.sh  # 拉最新 release
sudo systemctl restart sb-manager
```

配置文件 / 数据库不会被覆盖。

---

## 卸载

```bash
sudo systemctl disable --now sb-manager
sudo rm -f /usr/local/bin/sb /usr/bin/sb /etc/systemd/system/sb-manager.service /etc/profile.d/sb-manager.sh
# 如要一并清数据：
sudo rm -rf /etc/sing-box-manager /var/lib/sing-box-manager
# 如要卸载 sing-box 本体，进 TUI 内核页按 u；或：
sb kernel uninstall
```

---

## 文件位置

| 路径 | 用途 |
|---|---|
| `/usr/local/bin/sb` | sb-manager 主二进制 |
| `/etc/sing-box-manager/config.toml` | sb-manager 配置 |
| `/var/lib/sing-box-manager/manager.db` | SQLite 数据（用户、流量历史） |
| `/etc/systemd/system/sb-manager.service` | systemd unit |
| `/etc/profile.d/sb-manager.sh` | 清 stale alias |
| `/usr/local/bin/sing-box` | sing-box 内核二进制 |
| `/etc/sing-box/config.json` | sing-box 配置 |
| `/etc/systemd/system/sing-box.service` | sing-box systemd unit |

---

## 配置 `config.toml`

```toml
[singbox]
config_path = "/etc/sing-box/config.json"
binary_path = "/usr/local/bin/sing-box"
grpc_addr   = "127.0.0.1:18080"

[db]
path = "/var/lib/sing-box-manager/manager.db"

[stats]
sync_interval_secs  = 30    # 流量同步间隔
quota_alert_percent = 80    # 用户用量到达此百分比触发告警

[kernel]
# TUI 内核页「安装 v2ray_api 版」从此仓库 release 拉取
# 改成你自己的 fork 可用自编译版本
update_repo = "why1f/singbox-manager"

[subscription]
enabled     = true
listen      = "127.0.0.1:18081"           # 订阅 HTTP 监听（nginx 反代上游）
public_base = ""                          # 例: "https://sub.example.com"；填了才能拼对外订阅 URL
nginx_conf  = "/etc/nginx/conf.d/sb-manager.conf"
```

---

## 工作原理（极简）

```
┌──────────┐        ┌──────────┐        ┌─────────────┐
│   TUI    │◄──────►│ sb-mgr   │◄──gRPC►│  sing-box   │
│  (你)    │ UiEvent│ (daemon) │ 18080  │ +v2ray_api  │
└──────────┘        └──┬────┬──┘        └─────────────┘
                       │    │ sqlx
                       │    ▼
                       │  ┌──────────┐
                       │  │  SQLite  │ 用户、流量历史
                       │  └──────────┘
                       │ HTTP 18081
                       ▼
                   ┌──────────┐      ┌──────────┐
                   │ 订阅服务 │◄─反代│   nginx  │◄── 客户端拉订阅
                   └──────────┘      └──────────┘
```

- TUI 是客户端，一切改动走 `service/` 层写 SQLite + 重写 `/etc/sing-box/config.json`，然后 `systemctl reload sing-box`
- `sb daemon` 后台每 30 s 通过 gRPC 拉 sing-box 的用户流量统计，算增量写库；每分钟跑一次"到期禁用 / 月重置 / 超额禁用"
- 掉线指数退避重连，上限 60 s
- 订阅 HTTP 服务按 token 分发 sing-box / mihomo 订阅，**token 轮换**即可踢掉旧 URL
- 各协议订阅链接由 `service/sub_service.rs` 按 inbound 实际字段生成：自签证书自动带 `insecure=1`，reality/acme 不带；每个协议的字段与 sing-box schema 对齐

---

## 构建

```bash
cargo build --release    # 调试版 target/release/sb
cargo test
cargo clippy --all-targets -- -D warnings
```

Rust 1.74+，仅依赖系统 `protoc`（已内置 `protoc-bin-vendored`）。CI 用 musl 静态编译出 ~10 MB 二进制。

---

## 许可 / 依赖

本项目 Rust 代码遵循 MIT；安装时下载的 sing-box 二进制遵循其 [GPLv3](https://github.com/SagerNet/sing-box/blob/main/LICENSE)。

---

## 路线图 / 变更

- [ROADMAP.md](ROADMAP.md) 下一步计划
- [CHANGELOG.md](CHANGELOG.md) 版本变更
- [RELEASE_CHECKLIST.md](RELEASE_CHECKLIST.md) 发布流程

## 反馈

Issue / PR 欢迎。发布前跑：

```bash
cargo clippy --all-targets -- -D warnings
cargo test
```
