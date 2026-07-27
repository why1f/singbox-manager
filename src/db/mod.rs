pub mod tg_repo;
pub mod traffic_repo;
pub mod user_repo;

use anyhow::{Context, Result};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    ConnectOptions, Connection, Row, SqlitePool,
};
use std::str::FromStr;

/// 迁移脚本按 user_version 顺序执行；索引 i 对应目标版本 i+1。
const MIGRATIONS: &[&str] = &[
    include_str!("migrations/001_init.sql"),
    include_str!("migrations/002_allowed_nodes.sql"),
    include_str!("migrations/003_sub_token.sql"),
    include_str!("migrations/004_traffic_multiplier.sql"),
    include_str!("migrations/005_telegram.sql"),
    include_str!("migrations/006_auto_disabled.sql"),
];

/// 当前程序期望的 schema 版本（= 迁移脚本数量），供 doctor 比对实际库版本。
pub fn schema_version() -> i64 {
    MIGRATIONS.len() as i64
}

pub async fn init_pool(db_path: &str) -> Result<SqlitePool> {
    let url = format!("sqlite://{}?mode=rwc", db_path);

    // 迁移在**建池之前**、用一条独立连接跑完。
    // 若放在池上跑，先建好的连接可能缓存了 ALTER TABLE 之前的表结构，
    // 后续 `SELECT *` 会拿到列数与新结构不符的行（表现为解码时下标越界）。
    migrate(&url)
        .await
        .with_context(|| format!("迁移数据库 {} 失败", db_path))?;

    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .with_context(|| format!("打开数据库 {} 失败", db_path))?;
    sqlx::query("PRAGMA journal_mode=WAL")
        .execute(&pool)
        .await?;
    sqlx::query("PRAGMA synchronous=NORMAL")
        .execute(&pool)
        .await?;
    sqlx::query("PRAGMA busy_timeout=5000")
        .execute(&pool)
        .await?;
    sqlx::query("PRAGMA foreign_keys=ON").execute(&pool).await?;
    Ok(pool)
}

/// 线性迁移：每个版本的所有语句在一个事务里跑，要么整版生效要么整版回滚，
/// 不会停在"加了一半列"的中间态。ALTER TABLE 的重复列错误单独放行，
/// 兼容历史上被手工 ALTER 过的库。
async fn migrate(url: &str) -> Result<()> {
    let mut conn = SqliteConnectOptions::from_str(url)?
        .create_if_missing(true)
        .disable_statement_logging()
        .connect()
        .await?;
    sqlx::query("PRAGMA journal_mode=WAL")
        .execute(&mut conn)
        .await?;
    sqlx::query("PRAGMA busy_timeout=5000")
        .execute(&mut conn)
        .await?;

    let current: i64 = sqlx::query("PRAGMA user_version")
        .fetch_one(&mut conn)
        .await?
        .try_get(0)
        .unwrap_or(0);

    for (idx, script) in MIGRATIONS.iter().enumerate() {
        let target = idx as i64 + 1;
        if current >= target {
            continue;
        }
        let mut tx = conn.begin().await?;
        for stmt in split_sql(script) {
            if let Err(e) = sqlx::query(&stmt).execute(&mut *tx).await {
                if is_duplicate_column(&e) {
                    continue;
                }
                return Err(
                    anyhow::Error::new(e).context(format!("迁移 v{} 失败: {}", target, stmt))
                );
            }
        }
        // PRAGMA user_version 不接受绑定参数，target 来自枚举下标，非外部输入。
        sqlx::query(&format!("PRAGMA user_version = {}", target))
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
    }
    conn.close().await?;
    Ok(())
}

/// 旧库可能已被手工 ALTER 过同名列，这类错误视为幂等成功。
fn is_duplicate_column(e: &sqlx::Error) -> bool {
    e.to_string().contains("duplicate column")
}

fn split_sql(src: &str) -> Vec<String> {
    src.split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.lines().all(|l| l.trim_start().starts_with("--")))
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::split_sql;

    #[test]
    fn split_sql_drops_comment_only_chunks() {
        let src = "-- 注释\nALTER TABLE users ADD COLUMN a INTEGER;\n-- 尾部注释\n";
        let stmts = split_sql(src);
        assert_eq!(stmts.len(), 1);
        assert!(stmts[0].contains("ALTER TABLE"));
    }

    #[tokio::test]
    async fn migrations_run_to_latest_version_and_are_idempotent() {
        use sqlx::Row;
        let path = std::env::temp_dir().join(format!("sbm-mig-{}.db", uuid::Uuid::new_v4()));
        let url = format!("sqlite://{}?mode=rwc", path.to_string_lossy());
        let pool = super::init_pool(path.to_string_lossy().as_ref())
            .await
            .unwrap();
        let version: i64 = sqlx::query("PRAGMA user_version")
            .fetch_one(&pool)
            .await
            .unwrap()
            .try_get(0)
            .unwrap();
        assert_eq!(version, super::MIGRATIONS.len() as i64);
        // 再跑一次不应报错（幂等）
        super::migrate(&url).await.unwrap();
    }

    /// 迁移必须在建池之前跑完：池里任何一条连接若见过旧表结构，
    /// 后续 `SELECT *` 会拿到列数不匹配的行。这里用"建库→立刻插入并读回"验证。
    #[tokio::test]
    async fn fresh_database_can_round_trip_a_user() {
        let path = std::env::temp_dir().join(format!("sbm-rt-{}.db", uuid::Uuid::new_v4()));
        let pool = super::init_pool(path.to_string_lossy().as_ref())
            .await
            .unwrap();
        crate::service::user_service::add_user(&pool, "alice", 1.0, 0, "", 1.0)
            .await
            .unwrap();
        let got = super::user_repo::get(&pool, "alice").await.unwrap();
        assert!(got.is_some(), "刚插入的用户应能读回");
        assert!(!got.unwrap().auto_disabled);
    }
}
