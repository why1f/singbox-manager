ALTER TABLE users ADD COLUMN sub_token TEXT NOT NULL DEFAULT '';
CREATE UNIQUE INDEX IF NOT EXISTS idx_users_sub_token ON users(sub_token) WHERE sub_token != '';
