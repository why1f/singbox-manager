use crate::model::{
    traffic::{LiveTrafficSnapshot, TrafficDelta},
    user::User,
};

pub fn calc_deltas(snaps: &[LiveTrafficSnapshot], users: &[User]) -> Vec<TrafficDelta> {
    snaps
        .iter()
        .filter_map(|s| {
            let u = users.iter().find(|u| u.name == s.username)?;
            let du = calc_delta(s.up_bytes, u.last_live_up as u64);
            let dd = calc_delta(s.down_bytes, u.last_live_down as u64);
            if du == 0 && dd == 0 {
                return None;
            }
            Some(TrafficDelta {
                username: s.username.clone(),
                delta_up: du as i64,
                delta_down: dd as i64,
                new_live_up: s.up_bytes as i64,
                new_live_down: s.down_bytes as i64,
            })
        })
        .collect()
}

/// 防回绕：sing-box 重启后计数器清零，current < last 时直接用 current 为增量
#[inline]
pub fn calc_delta(current: u64, last: u64) -> u64 {
    if current >= last {
        current - last
    } else {
        current
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normal() {
        assert_eq!(calc_delta(1000, 600), 400);
    }
    #[test]
    fn no_change() {
        assert_eq!(calc_delta(600, 600), 0);
    }
    #[test]
    fn restart() {
        assert_eq!(calc_delta(200, 5000), 200);
    }
}
