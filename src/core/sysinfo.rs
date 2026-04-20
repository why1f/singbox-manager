/// 读取 Linux /proc 指标，用于 TUI 仪表盘。
/// Windows 上全部返回默认值。

#[derive(Debug, Clone, Default)]
pub struct CpuSample { pub idle: u64, pub total: u64 }

#[cfg(target_os = "linux")]
pub fn read_cpu() -> Option<CpuSample> {
    let s = std::fs::read_to_string("/proc/stat").ok()?;
    let line = s.lines().next()?;
    if !line.starts_with("cpu ") { return None; }
    let nums: Vec<u64> = line.split_whitespace().skip(1)
        .filter_map(|v| v.parse().ok()).collect();
    let idle = *nums.get(3)?;
    let total: u64 = nums.iter().sum();
    Some(CpuSample { idle, total })
}

#[cfg(not(target_os = "linux"))]
pub fn read_cpu() -> Option<CpuSample> { None }

/// 整机 rx/tx bytes（排除 lo）
#[cfg(target_os = "linux")]
pub fn read_net() -> Option<(u64, u64)> {
    let s = std::fs::read_to_string("/proc/net/dev").ok()?;
    let mut rx_total: u64 = 0;
    let mut tx_total: u64 = 0;
    for line in s.lines().skip(2) {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() != 2 { continue; }
        let iface = parts[0].trim();
        if iface == "lo" { continue; }
        let nums: Vec<u64> = parts[1].split_whitespace().filter_map(|v| v.parse().ok()).collect();
        let rx = *nums.first().unwrap_or(&0);
        let tx = *nums.get(8).unwrap_or(&0);
        rx_total += rx;
        tx_total += tx;
    }
    Some((rx_total, tx_total))
}

#[cfg(not(target_os = "linux"))]
pub fn read_net() -> Option<(u64, u64)> { None }

/// 计算 CPU 使用百分比 (0-100)，传入上次和本次采样
pub fn cpu_percent(prev: &CpuSample, cur: &CpuSample) -> u8 {
    let dt = cur.total.saturating_sub(prev.total);
    let di = cur.idle.saturating_sub(prev.idle);
    if dt == 0 { return 0; }
    let busy = dt.saturating_sub(di);
    ((busy as f64 / dt as f64) * 100.0).clamp(0.0, 100.0) as u8
}
