use crate::model::traffic::LiveTrafficSnapshot;
use anyhow::{anyhow, Result};
use std::time::Duration;
use tonic::transport::{Channel, Endpoint};

pub mod proto {
    tonic::include_proto!("v2ray.core.app.stats.command");
}
use proto::stats_service_client::StatsServiceClient;
use proto::QueryStatsRequest;
pub type StatsClient = StatsServiceClient<Channel>;

/// 建连超时。sing-box 半死（端口 accept 但不响应）时不能无限等——
/// 同步任务和自动控制跑在同一个 select 循环里，一次挂起会让月重置 / 到期禁用全部停摆。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// 单次 RPC 超时，理由同上。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

pub async fn connect(addr: &str) -> Result<StatsClient> {
    let ch = Endpoint::from_shared(format!("http://{}", addr))?
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .connect()
        .await
        .map_err(|e| anyhow!("连接 gRPC ({}) 失败: {}", addr, e))?;
    Ok(StatsServiceClient::new(ch))
}

pub async fn query_all_traffic(
    c: &mut StatsClient,
    reset: bool,
) -> Result<Vec<LiveTrafficSnapshot>> {
    let mut req = tonic::Request::new(QueryStatsRequest {
        pattern: "user>>>".into(),
        patterns: vec![],
        reset,
        regexp: false,
    });
    // Endpoint::timeout 只覆盖建连后的整体超时配置，这里再显式给单次请求设 deadline，
    // 保证服务端半死时 sync_once 一定会返回错误而不是永久挂起。
    req.set_timeout(REQUEST_TIMEOUT);
    let r = c
        .query_stats(req)
        .await
        .map_err(|e| anyhow!("QueryStats: {}", e))?;
    let mut map: std::collections::HashMap<String, (u64, u64)> = Default::default();
    for s in r.into_inner().stat {
        let p: Vec<&str> = s.name.split(">>>").collect();
        if p.len() != 4 || p[0] != "user" || p[2] != "traffic" {
            continue;
        }
        let e = map.entry(p[1].to_string()).or_default();
        let v = s.value.max(0) as u64;
        match p[3] {
            "uplink" => e.0 = v,
            "downlink" => e.1 = v,
            _ => {}
        }
    }
    Ok(map
        .into_iter()
        .map(|(n, (u, d))| LiveTrafficSnapshot {
            username: n,
            up_bytes: u,
            down_bytes: d,
        })
        .collect())
}
