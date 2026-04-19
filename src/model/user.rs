use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct User {
    pub name:            String,
    pub uuid:            String,
    pub password:        String,
    pub enabled:         bool,
    pub quota_gb:        f64,
    pub used_up_bytes:   i64,
    pub used_down_bytes: i64,
    pub manual_bytes:    i64,
    pub last_live_up:    i64,
    pub last_live_down:  i64,
    pub reset_day:       i64,
    pub last_reset_ym:   String,
    pub expire_at:       String,
    pub allow_all_nodes: bool,
    pub created_at:      String,
    #[sqlx(default)]
    pub allowed_nodes:   String,   // JSON 数组字符串：["tag1","tag2"]；空或 [] = 无
}

impl User {
    pub fn used_total_bytes(&self) -> i64 {
        self.used_up_bytes + self.used_down_bytes + self.manual_bytes
    }
    pub fn quota_bytes(&self) -> i64 { (self.quota_gb * 1_073_741_824.0) as i64 }
    pub fn quota_used_percent(&self) -> f64 {
        let q = self.quota_bytes();
        if q == 0 { return 0.0; }
        (self.used_total_bytes() as f64 / q as f64 * 100.0).min(100.0)
    }
    pub fn is_expired(&self) -> bool {
        if self.expire_at.is_empty() { return false; }
        NaiveDate::parse_from_str(&self.expire_at, "%Y-%m-%d")
            .map(|exp| chrono::Local::now().date_naive() > exp)
            .unwrap_or(false)
    }
    pub fn is_over_quota(&self) -> bool {
        let q = self.quota_bytes();
        q > 0 && self.used_total_bytes() >= q
    }
    pub fn allowed_tags(&self) -> Vec<String> {
        if self.allowed_nodes.is_empty() { return vec![]; }
        serde_json::from_str(&self.allowed_nodes).unwrap_or_default()
    }
    /// 是否允许访问指定 inbound tag
    pub fn can_use_node(&self, tag: &str) -> bool {
        if self.allow_all_nodes { return true; }
        self.allowed_tags().iter().any(|t| t == tag)
    }
    pub fn format_bytes(bytes: i64) -> String {
        const TB: i64 = 1_099_511_627_776;
        const GB: i64 = 1_073_741_824;
        const MB: i64 = 1_048_576;
        const KB: i64 = 1_024;
        match bytes {
            b if b >= TB => format!("{:.2} TB", b as f64 / TB as f64),
            b if b >= GB => format!("{:.2} GB", b as f64 / GB as f64),
            b if b >= MB => format!("{:.2} MB", b as f64 / MB as f64),
            b if b >= KB => format!("{:.2} KB", b as f64 / KB as f64),
            b            => format!("{} B", b),
        }
    }
}
