# singbox-manager 开发文档

## 当前发行建议

当前建议发行版本：

- `v0.1.0-beta`

当前发行材料：

- `README.md`
- `ROADMAP.md`
- `CHANGELOG.md`
- `RELEASE_CHECKLIST.md`

---

## 项目定位

`singbox-manager` 是一个面向 sing-box 的轻量管理工具，定位明确为：

- **轻量**
- **高性能**
- **CLI + TUI**
- **单机 / 小规模管理优先**
- **不做 Web，不做臃肿平台**

项目聚焦四个核心能力：

1. **节点搭建 / 部署**
2. **用户管理**
3. **流量统计**
4. **订阅导出**

---

## 明确不做的范围

当前项目目标下不做：

- Web 面板
- 浏览器控制台
- 多管理员协作
- 复杂权限系统
- 大型平台化扩展
- 与 sing-box 核心管理无关的冗杂能力

即：

**这是一个终端工具，不是 Web 后台。**

---

# 一、当前已开发功能

## 1. 工程基础

已完成：

- Rust 项目结构已恢复为真实工程
- `cargo check` 通过
- `cargo build --release` 通过
- `cargo clippy -- -D warnings` 通过
- 已内置 vendored `protoc`

说明：

- 当前项目已经具备稳定编译、构建和继续迭代能力

---

## 2. 配置加载与保存

已完成：

- 读取 `config.toml`
- 自动生成默认配置文件
- 支持保存 sing-box JSON 配置文件
- 支持基础配置项：
  - `singbox.config_path`
  - `singbox.binary_path`
  - `singbox.grpc_addr`
  - `db.path`
  - `stats.sync_interval_secs`
  - `stats.quota_alert_percent`

---

## 3. SQLite 数据存储

已完成：

- SQLite 初始化
- 自动建表
- 用户表
- 流量历史表
- WAL 模式优化
- 用户凭据字段支持：
  - `uuid`
  - `password`

当前数据库用于：

- 保存用户信息
- 保存凭据
- 保存累计流量
- 保存手动调整流量
- 保存到期时间
- 保存月重置信息
- 保存流量历史

---

## 4. 用户管理（CLI）

当前短命令已支持：

- `sb users`
- `sb add`
- `sb del`
- `sb on`
- `sb off`
- `sb reset`
- `sb info`
- `sb sub`
- `sb pkg`
- `sb add-traffic`

兼容旧命令：

- `sb user ...`

已完成逻辑：

- 用户名校验
- 用户创建/删除
- 启用/禁用切换
- 套餐更新
- 手动流量调整
- 流量重置
- 用户详情查询
- 自动生成用户凭据

---

## 5. 用户管理与 sing-box 配置联动

已完成：

- 用户变更后自动同步到 sing-box 配置中的 `users`
- 自动更新 `experimental.v2ray_api.stats.users`
- 自动保存配置
- 自动校验配置
- sing-box 运行中时自动 reload

说明：

- 这意味着用户管理不再只是改 SQLite，而是已经开始影响真实运行配置
- 这是项目从“原型”进入“可实际使用工具”的关键一步

---

## 6. 流量统计

已完成：

- 支持连接 sing-box V2Ray API
- 支持 `QueryStats`
- 支持解析 `user>>>...>>>traffic>>>uplink/downlink`
- 支持按用户计算流量增量
- 支持处理计数器回绕 / 重启清零
- 支持累计写入数据库
- 支持流量历史记录

说明：

- 当前统计链路已经成型
- 依赖 sing-box 正确启用 `experimental.v2ray_api`

---

## 7. 自动控制

已完成：

- 到期禁用
- 超额禁用
- 月重置
- 自动控制事件输出

说明：

- 已接入 daemon 后台流程
- 适合轻量管理工具场景的基础自动控制

---

## 8. daemon 后台模式

已完成：

- `sb daemon`
- gRPC 自动重连
- 周期同步流量
- 自动控制检查
- 后台日志输出
- 与 `systemd` 集成

说明：

- 这是 Linux 服务器上推荐的常驻运行模式

---

## 9. 服务管理命令

已完成：

- `sb check`
- `sb start`
- `sb stop`
- `sb reload`
- `sb status`

说明：

- 当前已具备 sing-box 基础服务管理闭环
- 已支持配置检查、启动、停止、重载、状态查看

---

## 10. 节点解析

已完成：

- 从 sing-box 配置读取 inbound
- 提取 tag
- 提取监听端口
- 提取协议类型
- 统计节点用户数

当前已识别协议：

- VLESS Reality
- VLESS WS
- VMess WS
- Trojan
- Shadowsocks
- Hysteria2
- TUIC
- AnyTLS

说明：

- 这里已经参考较完善项目中的节点配置格式做了增强
- 当前仍属于常见场景适配，不是对所有 sing-box 写法的完全兼容

---

## 11. 节点部署最小闭环

已完成：

- `sb add-node`
- 将新节点写入 sing-box `inbounds`
- 保存配置
- 校验配置
- sing-box 运行中时自动 reload

说明：

- 当前已具备“最小节点新增闭环”
- 这是轻量工具方向下的最小可发行节点部署能力
- 当前仍不是完整的节点编排系统

---

## 12. 订阅导出

已完成：

- 单用户明文链接导出
- Base64 订阅导出
- 多协议基础链接生成

当前已支持的链接类型：

- VLESS Reality
- VLESS WS
- VMess WS
- Trojan
- Shadowsocks
- Hysteria2
- TUIC
- AnyTLS

说明：

- 当前订阅导出已可以用于原型阶段和小规模实际使用
- 仍建议结合真实配置继续增强兼容性

---

## 13. TUI 终端界面

已完成：

- Dashboard 页面
- Users 页面
- Nodes 页面
- Logs 页面
- 页面切换
- 用户选择
- 状态栏显示
- 同步状态显示
- gRPC 状态显示
- sing-box 运行状态探测

当前 TUI 已支持：

- 浏览用户
- 浏览节点
- 浏览日志
- 查看同步状态
- 启用/禁用当前用户
- 重置当前用户流量
- 刷新当前状态
- 配置校验

说明：

- TUI 已经不只是浏览雏形，而是开始具备真实高频操作能力
- 复杂编辑能力仍建议保留给 CLI

---

## 14. Linux 部署基础

已完成：

- `install.sh`
- `sb-manager.service`
- release 构建安装
- 配置文件安装
- systemd 服务安装
- 启动前检查
- sing-box 可执行文件检查
- sing-box 配置文件检查

说明：

- 当前已具备基础 Linux 部署能力
- 更适合 Debian / Ubuntu 场景

---

# 二、当前仍待开发功能

## 1. 节点部署增强

待开发：

- 更多协议变种的节点生成
- 更细的 transport / tls / reality 参数支持
- 更完善的节点编辑能力
- 节点删除 / 节点修改命令

说明：

- 当前已有最小新增闭环
- 但还没有完整节点生命周期管理

---

## 2. 订阅导出兼容增强

待开发：

- 对更多真实 sing-box 配置写法做兼容
- 更细致处理 transport/tls/reality 参数
- 减少不同配置风格下的导出失败情况

---

## 3. TUI 交互继续增强

待开发：

- TUI 内直接导出订阅结果
- TUI 节点页操作
- TUI 服务页
- 更明显的操作反馈

说明：

- 当前 TUI 已具备一部分真实操作能力
- 还没完全成为全功能主面板

---

## 4. 配置兼容性继续增强

待开发：

- 更多 protocol / transport 识别
- 更稳的字段提取逻辑
- 更多真实 sing-box 配置风格兼容

---

## 5. 测试补强

待开发：

- 更多单元测试
- 数据层测试
- gRPC 测试
- 订阅导出测试
- 真实配置联调测试

---

# 三、当前是否可以发行使用

## 当前判断

**已经接近可以发行使用。**

如果按照你的项目目标：

- 轻量
- 高性能
- CLI + TUI
- 不做 Web
- 面向单机 / 小规模管理

那么当前项目已经具备：

- 编译稳定性
- 用户管理闭环
- 配置联动能力
- 服务管理能力
- 基础节点新增能力
- 流量统计能力
- 订阅导出能力
- 基础 TUI 操作能力

## 适合的发行场景

适合：

- 自用
- 单机服务器
- 小规模节点管理
- 内部工具
- 继续迭代型发行版本（0.x）

## 当前还不建议的场景

不建议直接用于：

- 大规模商用平台
- 多管理员复杂协作环境
- 要求极高配置兼容性的复杂生产网络

---

# 四、当前发行建议

当前更适合按如下方式发行：

## 推荐发行定位

- `0.x` 测试可用版
- 轻量自用工具版
- CLI/TUI 管理工具版

## 不建议宣称

- 不要宣称是成熟面板
- 不要宣称支持所有 sing-box 配置写法
- 不要宣称是企业级后台

---

# 五、下一阶段优先级

## P1

1. 节点删除 / 编辑
2. TUI 服务页 / 节点操作
3. 订阅导出兼容增强
4. 更多真实配置适配

## P2

5. 测试补强
6. 更多协议支持
7. 更细的诊断命令

---

# 六、常用命令

```bash
cargo check
cargo clippy -- -D warnings
cargo build --release
```

运行：

```bash
sb
sb tui
sb daemon
```

用户管理：

```bash
sb users
sb add <name> --quota 100 --reset-day 1 --expire 2026-12-31
sb del <name>
sb on <name>
sb off <name>
sb reset <name>
sb info <name>
sb sub <name>
sb pkg <name> --quota 200 --reset-day 1 --expire 2026-12-31
sb add-traffic <name> 10GB
```

节点与服务：

```bash
sb nodes
sb add-node <tag> --protocol vless-ws --port 443 --server-name example.com --path /ws
sb export <name>
sb check
sb start
sb stop
sb reload
sb status
```
