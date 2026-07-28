-- Telegram Bot 单实例租约。
--
-- 同一个 bot_token 只允许一个 getUpdates 长轮询：多开时 Telegram 会对其中一个
-- 返回 409 Conflict，两边都会随机丢 update，表现为"命令时灵时不灵"。
-- daemon 与 TUI 都可能启动 bot（例如服务在跑、管理员又开了 TUI），
-- 用这张表做跨进程互斥：拿到租约的实例才跑 bot，其余跳过。
--
-- 租约靠心跳续期；持有者进程被 kill 后心跳停止，超时即可被其他实例接管。
CREATE TABLE IF NOT EXISTS tg_bot_lease (
    id        INTEGER PRIMARY KEY CHECK (id = 1),
    owner     TEXT    NOT NULL,
    heartbeat TEXT    NOT NULL
);
