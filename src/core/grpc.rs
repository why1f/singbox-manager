use anyhow::{anyhow, Result};
use tonic::transport::Channel;
use crate::model::traffic::LiveTrafficSnapshot;

pub mod proto { tonic::include_proto!("v2ray.core.app.stats.command"); }
use proto::stats_service_client::StatsServiceClient;
use proto::QueryStatsRequest;
pub type StatsClient = StatsServiceClient<Channel>;

pub async fn connect(addr: &str) -> Result<StatsClient> {
    let ch = Channel::from_shared(format!("http://{}", addr))?
        .connect().await.map_err(|e| anyhow!("连接 gRPC ({}) 失败: {}", addr, e))?;
    Ok(StatsServiceClient::new(ch))
}

pub async fn query_all_traffic(c: &mut StatsClient, reset: bool) -> Result<Vec<LiveTrafficSnapshot>> {
    let r = c.query_stats(tonic::Request::new(QueryStatsRequest {
        pattern: "user>>>".into(), patterns: vec![], reset, regexp: false,
    })).await.map_err(|e| anyhow!("QueryStats: {}", e))?;
    let mut map: std::collections::HashMap<String,(u64,u64)> = Default::default();
    for s in r.into_inner().stat {
        let p: Vec<&str> = s.name.split(">>>").collect();
        if p.len()!=4 || p[0]!="user" || p[2]!="traffic" { continue; }
        let e = map.entry(p[1].to_string()).or_default();
        let v = s.value.max(0) as u64;
        match p[3] { "uplink"=>e.0=v, "downlink"=>e.1=v, _=>{} }
    }
    Ok(map.into_iter().map(|(n,(u,d))| LiveTrafficSnapshot{username:n,up_bytes:u,down_bytes:d}).collect())
}
