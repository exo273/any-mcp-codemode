//! Spike 001 — throwaway. Q5: one code path + one migration set for SQLite and Postgres via sqlx::Any.
use sqlx::{AnyPool, Row, any::install_default_drivers};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    install_default_drivers();
    for url in std::env::args().skip(1) {
        let pool = AnyPool::connect(&url).await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        for (i, ph) in [("a", "?"), ("b", "$1")].iter().enumerate() {
            let sql: &'static str = if ph.1 == "?" {
                "INSERT INTO executions (id, ts, code, status, result, duration_ms, tool_calls) VALUES (?, ?, ?, ?, ?, ?, ?)"
            } else {
                "INSERT INTO executions (id, ts, code, status, result, duration_ms, tool_calls) VALUES ($1, $2, $3, $4, $5, $6, $7)"
            };
            let r = sqlx::query(sql)
                .bind(format!("id-{}", ph.0)).bind("2026-11-21T23:50:00Z").bind("async () => 1")
                .bind("completed").bind(Some("{\"ok\":true}".to_string())).bind(12_i64 + i as i64).bind(3_i64)
                .execute(&pool).await;
            println!("{url:.40} placeholder {:3} -> {}", ph.1, r.map(|x| format!("ok rows={}", x.rows_affected())).unwrap_or_else(|e| format!("ERR {e}")));
        }
        // retention prune: one portable statement
        let pruned = sqlx::query("DELETE FROM executions WHERE id NOT IN (SELECT id FROM executions ORDER BY ts DESC, id DESC LIMIT 1)")
            .execute(&pool).await;
        println!("  prune -> {:?}", pruned.map(|x| x.rows_affected()));
        let rows = sqlx::query("SELECT id, duration_ms, result FROM executions ORDER BY ts DESC").fetch_all(&pool).await?;
        for r in rows {
            println!("  row id={} dur={} result={:?}", r.try_get::<String, _>(0)?, r.try_get::<i64, _>(1)?, r.try_get::<Option<String>, _>(2)?);
        }
        pool.close().await;
    }
    Ok(())
}
