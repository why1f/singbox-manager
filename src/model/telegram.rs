use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, sqlx::FromRow)]
pub struct TgAdminPrefs {
    pub chat_id: i64,
    #[sqlx(default)]
    pub notify_quota: bool,
    #[sqlx(default)]
    pub schedule_enabled: bool,
    #[sqlx(default)]
    pub schedule_times: String,
    #[sqlx(default)]
    pub last_schedule_dates: String,
}

impl TgAdminPrefs {
    pub fn schedule_times(&self) -> Vec<String> {
        serde_json::from_str(&self.schedule_times).unwrap_or_default()
    }

    pub fn last_schedule_dates(&self) -> std::collections::BTreeMap<String, String> {
        serde_json::from_str(&self.last_schedule_dates).unwrap_or_default()
    }
}
