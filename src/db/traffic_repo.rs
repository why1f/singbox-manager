use anyhow::Result;
use sqlx::SqlitePool;

pub async fn cleanup_old(pool: &SqlitePool) -> Result<u64> {
    Ok(sqlx::query("DELETE FROM traffic_history WHERE recorded_at < datetime('now','-30 days')")
        .execute(pool).await?.rows_affected())
}
