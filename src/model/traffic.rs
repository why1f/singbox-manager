use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct LiveTrafficSnapshot {
    pub username: String,
    pub up_bytes: u64,
    pub down_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrafficDelta {
    pub username: String,
    pub delta_up: i64,
    pub delta_down: i64,
    pub new_live_up: i64,
    pub new_live_down: i64,
}
