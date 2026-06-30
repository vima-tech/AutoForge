use anyhow::Result;
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use std::path::Path;

pub type Db = SqlitePool;

pub async fn init(db_path: &str) -> Result<Db> {
    let parent = Path::new(db_path).parent().unwrap_or(Path::new("."));
    tokio::fs::create_dir_all(parent).await?;
    let url = format!("sqlite://{}?mode=rwc", db_path);
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await?;
    sqlx::query("PRAGMA journal_mode=WAL")
        .execute(&pool)
        .await?;
    sqlx::query("PRAGMA busy_timeout=5000")
        .execute(&pool)
        .await?;
    sqlx::query("PRAGMA foreign_keys=ON").execute(&pool).await?;
    // 性能调优（WAL 下安全）：提交不再每次 fsync、加大页缓存、对读启用 mmap、临时表入内存。
    sqlx::query("PRAGMA synchronous=NORMAL")
        .execute(&pool)
        .await?;
    sqlx::query("PRAGMA cache_size=-16000")
        .execute(&pool)
        .await?;
    sqlx::query("PRAGMA mmap_size=268435456")
        .execute(&pool)
        .await?;
    sqlx::query("PRAGMA temp_store=MEMORY")
        .execute(&pool)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}
