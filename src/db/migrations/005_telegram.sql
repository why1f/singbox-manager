ALTER TABLE users ADD COLUMN tg_chat_id INTEGER NOT NULL DEFAULT 0;
ALTER TABLE users ADD COLUMN tg_bind_token TEXT NOT NULL DEFAULT '';
ALTER TABLE users ADD COLUMN tg_notify_quota_80 INTEGER NOT NULL DEFAULT 1;
ALTER TABLE users ADD COLUMN tg_notify_quota_90 INTEGER NOT NULL DEFAULT 1;
ALTER TABLE users ADD COLUMN tg_notify_quota_100 INTEGER NOT NULL DEFAULT 1;
ALTER TABLE users ADD COLUMN tg_schedule_enabled INTEGER NOT NULL DEFAULT 1;
ALTER TABLE users ADD COLUMN tg_schedule_times TEXT NOT NULL DEFAULT '[]';
ALTER TABLE users ADD COLUMN tg_last_quota_level INTEGER NOT NULL DEFAULT 0;
ALTER TABLE users ADD COLUMN tg_last_schedule_dates TEXT NOT NULL DEFAULT '{}';

CREATE TABLE IF NOT EXISTS tg_admin_prefs (
    chat_id              INTEGER PRIMARY KEY,
    notify_quota         INTEGER NOT NULL DEFAULT 1,
    schedule_enabled     INTEGER NOT NULL DEFAULT 1,
    schedule_times       TEXT    NOT NULL DEFAULT '[]',
    last_schedule_dates  TEXT    NOT NULL DEFAULT '{}'
);
