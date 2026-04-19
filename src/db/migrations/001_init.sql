CREATE TABLE IF NOT EXISTS users (
    name            TEXT    PRIMARY KEY,
    uuid            TEXT    NOT NULL DEFAULT '',
    password        TEXT    NOT NULL DEFAULT '',
    enabled         INTEGER NOT NULL DEFAULT 1,
    quota_gb        REAL    NOT NULL DEFAULT 0,
    used_up_bytes   INTEGER NOT NULL DEFAULT 0,
    used_down_bytes INTEGER NOT NULL DEFAULT 0,
    manual_bytes    INTEGER NOT NULL DEFAULT 0,
    last_live_up    INTEGER NOT NULL DEFAULT 0,
    last_live_down  INTEGER NOT NULL DEFAULT 0,
    reset_day       INTEGER NOT NULL DEFAULT 0,
    last_reset_ym   TEXT    NOT NULL DEFAULT '',
    expire_at       TEXT    NOT NULL DEFAULT '',
    allow_all_nodes INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT    NOT NULL
);
CREATE TABLE IF NOT EXISTS traffic_history (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    username    TEXT    NOT NULL,
    up_bytes    INTEGER NOT NULL DEFAULT 0,
    down_bytes  INTEGER NOT NULL DEFAULT 0,
    recorded_at TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_traffic_user ON traffic_history(username, recorded_at);
