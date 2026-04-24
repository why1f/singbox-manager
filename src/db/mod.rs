pub mod traffic_repo;
pub mod user_repo;

use anyhow::{Context, Result};
use sqlx::{sqlite::SqlitePoolOptions, Row, SqlitePool};

const SCHEMA_V1: &str = include_str!("migrations/001_init.sql");
const SCHEMA_V2: &str = include_str!("migrations/002_allowed_nodes.sql");
const SCHEMA_V3: &str = include_str!("migrations/003_sub_token.sql");
const SCHEMA_V4: &str = include_str!("migrations/004_traffic_multiplier.sql");

pub async fn init_pool(db_path: &str) -> Result<SqlitePool> {
    let url = format!("sqlite://{}?mode=rwc", db_path);
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
    migrate(&pool).await?;
    Ok(pool)
}

async fn migrate(pool: &SqlitePool) -> Result<()> {
    let version: i64 = sqlx::query("PRAGMA user_version")
        .fetch_one(pool)
        .await?
        .try_get(0)
        .unwrap_or(0);
    if version < 1 {
        for stmt in split_sql(SCHEMA_V1) {
            sqlx::query(&stmt)
                .execute(pool)
                .await
                .with_context(|| format!("迁移 v1 失败: {}", stmt))?;
        }
        sqlx::query("PRAGMA user_version = 1").execute(pool).await?;
    }
    if version < 2 {
        for stmt in split_sql(SCHEMA_V2) {
            // 旧库可能已有此列（被 ALTER 加过），忽略重复列错误
            if let Err(e) = sqlx::query(&stmt).execute(pool).await {
                let msg = e.to_string();
                if !msg.contains("duplicate column") {
                    return Err(e.into());
                }
            }
        }
        sqlx::query("PRAGMA user_version = 2").execute(pool).await?;
    }
    if version < 3 {
        for stmt in split_sql(SCHEMA_V3) {
            if let Err(e) = sqlx::query(&stmt).execute(pool).await {
                let msg = e.to_string();
                if !msg.contains("duplicate column") {
                    return Err(e.into());
                }
            }
        }
        sqlx::query("PRAGMA user_version = 3").execute(pool).await?;
    }
    if version < 4 {
        for stmt in split_sql(SCHEMA_V4) {
            if let Err(e) = sqlx::query(&stmt).execute(pool).await {
                let msg = e.to_string();
                if !msg.contains("duplicate column") {
                    return Err(e.into());
                }
            }
        }
        sqlx::query("PRAGMA user_version = 4").execute(pool).await?;
    }
    Ok(())
}

fn split_sql(src: &str) -> Vec<String> {
    src.split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}
