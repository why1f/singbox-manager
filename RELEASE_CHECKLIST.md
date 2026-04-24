# 发版检查清单

## 1. 构建检查

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo build --locked`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test --all`
- [ ] release workflow 的目标平台本地或 CI 至少跑通一次

## 2. 版本一致性

- [ ] `Cargo.toml` 版本正确
- [ ] `Cargo.lock` 已同步
- [ ] `CHANGELOG.md` 已写本次版本
- [ ] tag 与提交指向确认无误

## 3. 基础环境检查

- [ ] `sb doctor`
- [ ] `sb check`
- [ ] `sb status`
- [ ] sing-box 配置路径正确
- [ ] gRPC 地址正确
- [ ] `experimental.v2ray_api` 已启用
- [ ] 证书路径存在
- [ ] 订阅 `public_base` / nginx conf 配置正确

## 4. 服务管理检查

- [ ] `sb start`
- [ ] `sb stop`
- [ ] `sb reload`
- [ ] `sb status` 输出正常

## 5. 用户管理检查

- [ ] `sb add <name>`
- [ ] `sb info <name>`
- [ ] `sb on <name>`
- [ ] `sb off <name>`
- [ ] `sb reset <name>`
- [ ] `sb pkg <name> ...`
- [ ] `sb del <name>`

## 6. 配置联动检查

- [ ] 用户新增后写入 sing-box `users`
- [ ] 用户启禁后配置同步成功
- [ ] 用户变更后配置可通过 `sb check`
- [ ] sing-box 运行时用户变更后可自动 reload

## 7. 节点与订阅检查

- [ ] `sb nodes`
- [ ] `sb add-node ...`
- [ ] 新节点写入 `inbounds`
- [ ] `sb sub <name>`
- [ ] `sb export <name>`
- [ ] 客户端可识别导出的链接/订阅

## 8. 流量统计检查

- [ ] gRPC 可连接
- [ ] 用户流量可累计
- [ ] daemon 模式下可周期同步
- [ ] 日志中可看到同步 / 告警 / 自动控制输出

## 9. TUI 检查

- [ ] 页面切换正常
- [ ] 用户列表显示正常
- [ ] 节点列表显示正常
- [ ] 日志页显示正常
- [ ] `t` 启禁用户生效
- [ ] `r` 重置流量生效
- [ ] `R` 刷新状态生效
- [ ] `c` 配置校验生效
- [ ] `s` 可在日志页看到最近一次订阅导出

## 10. Linux 部署检查

- [ ] `install.sh` 可执行
- [ ] `install-release.sh` 可执行
- [ ] `sb-manager.service` 安装成功
- [ ] `systemctl start sb-manager.service`
- [ ] `systemctl status sb-manager.service`
- [ ] `journalctl -u sb-manager.service -f`
