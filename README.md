# singbox-manager

面向 sing-box 的**轻量 CLI + TUI 管理工具**（Rust）。不做 Web，不做平台，只干五件事：节点搭建 / 用户管理 / 流量统计 / 订阅导出 / Telegram 通知。

- **单机/小规模** 一台机一人管，别装一堆东西
- **静态 musl 二进制** ~10 MB，不依赖 glibc 版本
- **内核管理内置** 在 TUI 里一键装官方 sing-box 或带 `with_v2ray_api` 的自编译版
- **Telegram Bot 内置** 用户自助查流量/订阅，管理员远程管账号

---

## 安装（推荐：预编译二进制）

> 适用：Linux amd64 / arm64（任意发行版），只需 `curl` + `tar` + `systemd`，无需 Rust/gcc。

```bash
curl -fsSL https://raw.githubusercontent.com/why1f/singbox-manager/master/install-release.sh | sudo bash
sudo systemctl start sb-manager
```

脚本会：
1. 下载最新 `sb` 二进制到 `/etc/sing-box/bin/sb`
2. 写 `/etc/sing-box/manager/config.toml`、`/etc/systemd/system/sb-manager.service`
3. 建软链 `/usr/local/bin/sb` + 清 `/etc/profile.d/sb-manager.sh` 里的 stale alias
4. `systemctl enable sb-manager`

指定版本 / 自己的 fork：

```bash
curl -fsSL https://raw.githubusercontent.com/why1f/singbox-manager/master/install-release.sh \
  | sudo REPO=why1f/singbox-manager VERSION=v0.2.4 bash
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

daemon/TUI 会起一个本地订阅 HTTP（默认 `127.0.0.1:18081`），给每个用户分配一个 token，前挂 nginx 反代即可对外分发订阅。**浏览器直接访问 `/sub/<token>` 会看到带进度条、节点 QR 码的流量统计页**；代理客户端按 User-Agent 自动取对应格式。

```bash
sb token show alice                             # 打印订阅 URL + token
sb token regen alice                            # 轮换 token（旧 URL 立即失效）
sb token revoke alice                           # 撤销 token（关闭订阅，/sub/ 返回 404；再 regen 可恢复）
```

订阅 URL 的三种用法：

| 场景 | URL |
|---|---|
| 浏览器看流量 / 复制订阅 / 单节点 QR | `https://sub.example.com/sub/<token>` |
| 复制给 mihomo/Clash Meta | `https://sub.example.com/sub/<token>?type=clash` |
| 复制给 sing-box/v2rayN | `https://sub.example.com/sub/<token>?type=sing-box` |

其中 mihomo 等客户端用自己的 UA 拉订阅会自动拿到 yaml，不必手写 `?type=`；显式参数优先级最高。

> token 走 URL 路径，会留在 nginx access log 里。生产环境建议对 `/sub/` 关掉 access_log，或用
> `log_format` 把路径脱敏。

### Telegram 绑定码

```bash
sb bind list                                    # 所有用户的绑定状态一览
sb bind show alice                              # 单个用户的绑定状态 / 当前绑定码
sb bind regen alice                             # 生成（或重置）绑定码，旧码立即失效
sb bind unbind alice                            # 解除已建立的绑定
```

**绑定码是一次性的**：用户 `/bind <码>` 成功后立即作废，需要再绑必须重新 `sb bind regen`。

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

### 中转节点

落地机线路不好时，可以在前面挂一台中转机做转发。给节点填上中转地址后，**订阅里该节点的地址和端口会换成中转机的**，其余参数（SNI、凭据、传输方式）保持不变：

```bash
sb add-node hk -p vless-reality --port 4432 --relay-host relay.example.com --relay-port 12345
sb node edit hk --relay-host 203.0.113.9          # 只改地址，端口沿用节点端口
sb node edit hk --relay-host ""                   # 关闭中转，恢复导出本机地址
```

TUI 的添加/编辑节点表单里对应「中转 IP/域名」和「中转端口」两栏，节点列表和仪表盘都会标出 `→ 中转地址`。

| | 未设中转 | 设了中转 |
|---|---|---|
| 订阅里的 server | 本机公网 IP（或 `public_base` 主机） | 中转机地址 |
| 订阅里的 port | 节点端口（端口复用时为 443） | 中转端口，留空则同左 |
| SNI / 凭据 / 传输 | — | 不变 |
| sing-box `config.json` | — | 不变 |

> **转发本身要你自己在中转机上搭**（iptables DNAT、socat、gost、realm 等均可）。本工具只负责把订阅指向中转机，不涉及转发的实现与部署。
> 中转地址是 IPv6 字面量时直接填 `2001:db8::1`，导出时会自动补方括号。

### sing-box 内核

```bash
sb kernel status
sb kernel install                   # 官方版
sb kernel install-v2ray-api         # v2ray_api 版（推荐）
sb kernel start / stop / restart
sb kernel enable / disable          # 开机自启
sb kernel uninstall
sb doctor                           # 一键自检部署状态
```

装完会自动探测一次 gRPC 统计接口：装官方版时会明确提示"流量统计不可用"，不用等到发现流量一直是 0。

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
sb doctor                           # 检查 config / db / gRPC / v2ray_api / 证书 / nginx
```

`sb doctor` 会输出 `OK / WARN / ERR` 三类结果，重点检查：
- `config.toml` / `config.json` 是否存在且可解析
- 数据库是否可读写、**结构版本是否跟得上当前二进制**
- sing-box 二进制路径、`sing-box check`、gRPC 连通性
- `experimental.v2ray_api` 是否启用且地址匹配
- TLS 引用的证书/私钥文件是否存在
- 订阅配置和 nginx `-t` 是否正常

---

## Telegram Bot

内置的 TG bot 让用户自助查流量/取订阅、管理员远程管账号，不必登服务器。**daemon 与 TUI 模式都会启动它**。

### 配置

1. 找 [@BotFather](https://t.me/BotFather) 建 bot 拿 token
2. 找 [@userinfobot](https://t.me/userinfobot) 拿自己的 chat_id（管理员用）
3. 填 `config.toml`：

```toml
[telegram]
enabled        = true
bot_token      = "123456:ABC-DEF..."
admin_chat_ids = [123456789]          # 管理员 chat_id，可多个
timezone       = "Asia/Shanghai"      # 见下方"时区"说明
```

4. `systemctl restart sb-manager`

### 让用户绑定

```bash
sb bind regen alice          # 生成绑定码
```

把输出的 `/bind <码>` 发给用户，他在 bot 里发一次即可。**绑定码一次性**，用过即废；泄露了直接 `sb bind regen` 换一个。也可以在 bot 的管理员面板里点「用户列表 → 某用户 → TG 绑定」完成同样操作。

绑定成功后该用户的绑定码会被清空，这是正常状态——需要重新绑定时再 `sb bind regen` 生成新码即可。

### 命令与面板

| 命令 | 谁能用 | 作用 |
|---|---|---|
| `/start` | 所有人 | 主菜单：已绑定→个人面板，管理员→管理面板 |
| `/bind <绑定码>` | 持码者 | 绑定账号 |
| `/usage` | 已绑定用户 | 我的流量 |
| `/usages` | 仅管理员 | 全部用户流量汇总 |

**用户面板**（按钮）：我的流量 / 我的订阅（URL、Base64、明文节点）/ 刷新流量（实时拉一次 gRPC）/ 通知设置（80%、90%、100% 三档阈值开关 + 定时播报时间）

**管理员面板**：用户概览（总数/启用/停用/超额/到期/已绑定/整体流量）、全部用户流量、用户列表（点进去可查流量、导订阅、管绑定、刷新）、管理员通知设置

### 通知

- **阈值告警**：用量到 80% / 90% / 100% 各推一次，只升不重发；月重置后自动复位，下个周期可再触发
- **定时播报**：用户和管理员各自可设时间（默认 `09:00` / `21:30`），按 `[telegram].timezone` 计时

> **同一个 bot_token 只能有一个进程轮询**。daemon 和 TUI 都会尝试启动 bot，
> 由一张数据库租约表决定谁来跑：daemon 在运行时打开 TUI，TUI 会跳过启动并在日志里说明。
> 如果两台机器共用了同一个 token，Telegram 会返回 409，程序会停止轮询并给出提示。

### 时区

支持 `±HH:MM` 偏移和**无夏令时**的 IANA 名（`Asia/Shanghai`、`Asia/Tokyo`、`Australia/Brisbane` 等）。
有夏令时的时区（`Europe/London`、`America/New_York` …）**会被故意拒绝**并回落 `+08:00` 且打 warn——固定偏移在 DST 期间会偏 1 小时，静默算错比明确拒绝更糟。这类时区请直接填偏移量，如 `+00:00` / `-05:00`。

### 完整配置项

```toml
[telegram]
enabled                 = false
bot_token               = ""
timezone                = "Asia/Shanghai"
admin_chat_ids          = []
poll_interval_secs      = 2       # 无更新时的轮询间隔（getUpdates 本身是 25s long-poll）
request_timeout_secs    = 10
default_notify_quota_80 = true    # 新建用户的默认阈值开关
default_notify_quota_90 = true
default_notify_quota_100 = true
default_schedule_enabled = true
default_schedule_times  = ["09:00", "21:30"]
admin_notify_quota      = true    # 管理员默认收阈值告警
admin_schedule_enabled  = true
admin_schedule_times    = ["09:00", "21:30"]
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

**用户页**：`[a]` 添加 `[E]` 编辑 `[d]` 删除 `[t]` 启禁 `[r]` 重置流量 `[T]` token 管理（生成 / 撤销）`[u]` 复制订阅 URL `[s]` 打印订阅链接 `[n]` 分配可用节点

**节点页**：`[a]` 添加 `[E]` 编辑 `[d]` 删除（弹窗表单按当前协议显示字段：reality/trojan/tuic/anytls 多一行 `server_name`；vless-ws/vmess-ws 多一行 `path`；hysteria2/shadowsocks 只要端口；reality/trojan/anytls 编辑时还多一个"端口复用"开关；所有协议都有"订阅优先 IPv6"和"中转 IP/域名 + 中转端口"两组可选项）。列表里 `中转` 一列会显示 `→ 地址:端口`。

**内核页**：`[i]` 装官方 `[v]` 装 v2ray_api 版 `[u]` 卸载 `[s/S/x]` 启/停/重启 `[e/d]` 开/关自启

**nginx 页**：`[i]` 装 `[g]` 生成反代 conf `[t]` 语法检查 `[s/S/x]` 启/停/重启 `[r]` reload `[e/d]` 开/关自启

添加节点后会自动 `sing-box check -c ... && systemctl reload sing-box`，校验不通过不会覆盖。

## 维护建议

- 发版前至少跑一次 `cargo fmt --all -- --check`
- 确认 `cargo build --locked`、`cargo clippy --all-targets -- -D warnings`、`cargo test --all` 都通过
- 部署机上跑一次 `sb doctor`，比单独看 `sb status` 更容易发现缺失证书、gRPC 不通、`v2ray_api` 没开这类问题
- 升级二进制后记得 `systemctl restart sb-manager`：数据库迁移是启动时跑的，不重启的话 `sb doctor` 会报"结构版本落后"

---

## 端口复用（Reality / Trojan / anyTLS 共享 443）

vless-reality 跑在 443 上对"伪装正常网站"很有帮助，但订阅 HTTP 也想用 443 时就冲突了。这种场景用 nginx stream + `ssl_preread` 按 SNI 分流。

**TUI 操作**：选中节点 → `[E]` → 把"端口复用"切到"开" → 保存。工具会自动：

- sing-box inbound 的 `listen` 改写为 `127.0.0.1`（只对回环可见，由 nginx 回源）
- 订阅 URL 的 port 写死 443（不再跟着 inbound 的 `listen_port`）

**nginx 侧要你自己配**，参考模板：

```nginx
# Layer 4: SNI 分流
stream {
    map $ssl_preread_server_name $upstream {
        www.apple.com   127.0.0.1:4433;   # Reality 假域名 → sing-box inbound
        506777.xyz      127.0.0.1:8443;   # 你的真域名 → 下面 https 块
        default         127.0.0.1:8443;
    }
    server {
        listen 443 reuseport;
        listen [::]:443 reuseport;
        ssl_preread on;
        proxy_pass $upstream;
        proxy_timeout 300s;
    }
}

http {
    server {
        listen 127.0.0.1:8443 ssl http2;
        server_name 506777.xyz;
        ssl_certificate     /etc/nginx/ssl/506777.xyz.crt;
        ssl_certificate_key /etc/nginx/ssl/506777.xyz.key;

        location ~ "^/sub/[A-Za-z0-9_-]{16,64}$" {
            proxy_pass http://127.0.0.1:18081;
            proxy_set_header Host $host;
        }
        location / { return 404; }
    }
}
```

> 不加 `proxy_protocol`：sing-box 从 1.11 起 inbound 不再支持接收 PROXY header。代价是 nginx 和 sing-box 日志看到的 `remote_addr` 都是 127.0.0.1，但订阅 token 鉴权、sing-box 流量统计都和客户端 IP 无关，**功能上无影响**。

支持端口复用的协议：**vless-reality / trojan / anytls**（都是 TCP+TLS+SNI）。hy2/tuic 是 UDP 走不了 L4 preread；ss / vless-ws / vmess-ws 没 SNI 也不支持。

---

## 升级

```bash
curl -fsSL https://raw.githubusercontent.com/why1f/singbox-manager/master/install-release.sh | sudo bash
sudo systemctl restart sb-manager
```

配置文件 / 数据库不会被覆盖。

---

## 卸载

```bash
sudo systemctl disable --now sb-manager
sudo rm -f /usr/local/bin/sb /etc/systemd/system/sb-manager.service /etc/profile.d/sb-manager.sh
# 如要一并清数据：
sudo rm -rf /etc/sing-box
# 如要卸载 sing-box 本体，进 TUI 内核页按 u；或：
sb kernel uninstall
```

---

## 文件位置

| 路径 | 用途 |
|---|---|
| `/etc/sing-box/bin/sb` | sb-manager 主二进制 |
| `/etc/sing-box/manager/config.toml` | sb-manager 配置 |
| `/etc/sing-box/manager/manager.db` | SQLite 数据（用户、流量历史） |
| `/etc/sing-box/manager/nodes.meta.json` | 节点导出元数据（reality 公钥、SS 密码、端口复用、IPv6 偏好） |
| `/etc/sing-box/certs/` | 各节点的自签证书 |
| `/etc/sing-box/backup/` | 备份归档 |
| `/etc/systemd/system/sb-manager.service` | systemd unit |
| `/etc/profile.d/sb-manager.sh` | 清 stale alias |
| `/etc/sing-box/bin/sing-box` | sing-box 内核二进制 |
| `/etc/sing-box/config.json` | sing-box 配置 |
| `/etc/systemd/system/sing-box.service` | sing-box systemd unit |

---

## 配置 `config.toml`

```toml
[singbox]
config_path = "/etc/sing-box/config.json"
binary_path = "/etc/sing-box/bin/sing-box"
grpc_addr   = "127.0.0.1:18080"

[db]
path = "/etc/sing-box/manager/manager.db"

[stats]
sync_interval_secs  = 30    # 流量同步间隔
quota_alert_percent = 80    # 告警首档；另外两档固定 90 / 100

[kernel]
# TUI 内核页「安装 v2ray_api 版」从此仓库 release 拉取
# 改成你自己的 fork 可用自编译版本
update_repo = "why1f/singbox-manager"

[subscription]
enabled     = true
listen      = "127.0.0.1:18081"           # 订阅 HTTP 监听（nginx 反代上游）
public_base = ""                          # 例: "https://sub.example.com"；填了才能拼对外订阅 URL
# 订阅里节点的 server 是否跟随 public_base 主机；默认 false，直接导出公网 IP
use_public_base_as_server = false
nginx_conf  = "/etc/nginx/conf.d/sb-manager.conf"

# Telegram Bot（可选），见上方「Telegram Bot」章节
[telegram]
enabled   = false
bot_token = ""
admin_chat_ids = []
```

`grpc_addr` 改了之后不必手工改 `config.json`——下一次用户同步会自动把 `experimental.v2ray_api.listen` 对齐过去。

---

## 工作原理（极简）

```
┌──────────┐        ┌──────────┐        ┌─────────────┐
│   TUI    │◄──────►│ sb-mgr   │◄──gRPC►│  sing-box   │
│  (你)    │ UiEvent│ (daemon) │ 18080  │ +v2ray_api  │
└──────────┘        └─┬──┬──┬──┘        └─────────────┘
                      │  │  │ sqlx
                      │  │  ▼
                      │  │ ┌──────────┐
                      │  │ │  SQLite  │ 用户、流量历史
                      │  │ └──────────┘
                      │  │ HTTP 18081
                      │  ▼
                      │ ┌──────────┐      ┌──────────┐
                      │ │ 订阅服务 │◄─反代│   nginx  │◄── 客户端拉订阅
                      │ └──────────┘      └──────────┘
                      │ long-poll
                      ▼
                  ┌──────────┐
                  │ TG Bot   │◄── 用户查流量 / 管理员管账号
                  └──────────┘
```

- TUI 是客户端，一切改动走 `service/` 层写 SQLite + 重写 `/etc/sing-box/config.json`，然后 `systemctl reload sing-box`
- 配置落盘是 **写 `.tmp` → `sing-box check` 预校验 → 原子 rename**，坏配置永远不会覆盖主路径；跨进程用 SQLite `BEGIN IMMEDIATE` 当写锁串行化
- `sb daemon` 后台每 30 s 通过 gRPC 拉 sing-box 的用户流量统计，算增量写库；每分钟跑一次"到期禁用 / 月重置 / 超额禁用"
- 掉线指数退避重连，上限 60 s；**gRPC 断开期间自动控制照常执行**，恢复后补同步配置
- 收到 SIGTERM / Ctrl-C 会**先把内存里的流量增量落库再退出**，`systemctl stop` 不丢最后一个同步周期的数据
- 订阅 HTTP 服务按 token 分发 sing-box / mihomo 订阅，**token 轮换**即可踢掉旧 URL
- 各协议订阅链接由 `service/sub_service.rs` 按 inbound 实际字段生成：自签证书自动带 `insecure=1`，reality/acme 不带；每个协议的字段与 sing-box schema 对齐

### 启用 / 禁用的两种来源

`enabled` 之外还有一个 `auto_disabled` 标记，用来区分谁把用户停掉的：

| 场景 | auto_disabled | 月重置日会否自动解封 |
|---|---|---|
| 超额被系统禁用 | 1 | ✅ 会 |
| 到期被系统禁用 | 1 | ❌ 不会（到期判断优先） |
| `sb off` / TUI `[t]` 手动停用 | 0 | ❌ 不会 |

手动 `sb reset` 清零流量时，**只有超额自动禁用的用户**会立即恢复可用。

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
