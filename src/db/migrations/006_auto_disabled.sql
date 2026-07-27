-- 区分"自动禁用"与"管理员手动禁用"：
-- 只有 auto_disabled=1（超额被系统禁用）的用户才会在月重置日自动解封，
-- 管理员手动 sb off / TUI [t] 停用的用户不会被重置日误恢复。
ALTER TABLE users ADD COLUMN auto_disabled INTEGER NOT NULL DEFAULT 0;

-- 老库回填：升级前"停用且已超额"的用户，几乎必然是被自动控制禁掉的。
-- 不回填的话它们会被当成手动停用，下个重置日流量清零却不解封——
-- 相对 v0.4.x 是行为倒退。到期停用的用户即使被标上也无妨：
-- 自动控制里到期分支先 continue，走不到解封那一步。
UPDATE users SET auto_disabled = 1
WHERE enabled = 0
  AND quota_gb > 0
  AND (used_up_bytes + used_down_bytes) * COALESCE(traffic_multiplier, 2.0)
      >= quota_gb * 1073741824;

-- 004 迁移遗留：traffic_multiplier 是唯一没有 NOT NULL 的数值列，
-- 外部工具写入 NULL 会让 FromRow 解码失败并连带打挂所有用户列表查询。
UPDATE users SET traffic_multiplier = 2.0 WHERE traffic_multiplier IS NULL;
