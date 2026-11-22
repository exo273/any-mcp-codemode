//! Execution history in SQLite (default) or Postgres, through one `sqlx::Any` code path.
//! All SQL uses `$n` placeholders (works on both engines; `?` fails on Postgres).

use crate::sandbox::Outcome;
use sqlx::{AnyPool, Row, any::install_default_drivers};
use std::path::Path;

/// Execution store. Cloning shares the pool.
#[derive(Clone)]
pub struct Store {
    pool: AnyPool,
    keep: i64,
}

/// One stored execution (for `history`).
#[derive(Debug, serde::Serialize)]
pub struct Row_ {
    /// Execution id.
    pub id: String,
    /// RFC 3339 timestamp.
    pub ts: String,
    /// Outcome status.
    pub status: String,
    /// Host tool calls.
    pub tool_calls: i64,
    /// Duration in ms.
    pub duration_ms: i64,
    /// Submitted code.
    pub code: String,
}

/// Build a SQLite URL from a filesystem path (never concatenate raw `sqlite://C:/...`).
#[must_use]
pub fn sqlite_url(path: &Path) -> String {
    format!(
        "sqlite:{}?mode=rwc",
        path.to_string_lossy().replace('\\', "/")
    )
}

impl Store {
    /// Open (and migrate) the store. `keep` = executions retained (pruned on insert).
    pub async fn open(url: &str, keep: i64) -> Result<Self, sqlx::Error> {
        install_default_drivers();
        let pool = AnyPool::connect(url).await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool, keep })
    }

    /// Record one execution and prune old ones.
    pub async fn record(&self, code: &str, o: &Outcome) -> Result<String, sqlx::Error> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let id = format!(
            "{:x}{:08x}",
            now.as_millis(),
            std::process::id() ^ now.subsec_nanos()
        );
        sqlx::query("INSERT INTO executions (id, ts, code, status, result, error, logs, tool_calls, duration_ms) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)")
            .bind(&id)
            .bind(rfc3339(now.as_secs()))
            .bind(code)
            .bind(o.status)
            .bind(Some(o.result.to_string()))
            .bind(o.error.clone())
            .bind(serde_json::to_string(&o.logs).unwrap_or_default())
            .bind(i64::try_from(o.tool_calls).unwrap_or(i64::MAX))
            .bind(i64::try_from(o.duration_ms).unwrap_or(i64::MAX))
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM executions WHERE id NOT IN (SELECT id FROM executions ORDER BY ts DESC, id DESC LIMIT $1)")
            .bind(self.keep)
            .execute(&self.pool)
            .await?;
        Ok(id)
    }

    /// Latest executions, newest first.
    pub async fn history(&self, limit: i64) -> Result<Vec<Row_>, sqlx::Error> {
        let rows = sqlx::query("SELECT id, ts, status, tool_calls, duration_ms, code FROM executions ORDER BY ts DESC, id DESC LIMIT $1")
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(|r| {
                Ok(Row_ {
                    id: r.try_get(0)?,
                    ts: r.try_get(1)?,
                    status: r.try_get(2)?,
                    tool_calls: r.try_get(3)?,
                    duration_ms: r.try_get(4)?,
                    code: r.try_get(5)?,
                })
            })
            .collect()
    }
}

/// Seconds since epoch → `YYYY-MM-DDTHH:MM:SSZ` (civil-from-days, no date crate needed).
#[must_use]
pub fn rfc3339(secs: u64) -> String {
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn outcome(status: &'static str) -> Outcome {
        Outcome {
            status,
            result: json!({"ok": true}),
            error: None,
            logs: vec!["l".into()],
            tool_calls: 2,
            duration_ms: 7,
        }
    }

    #[test]
    fn rfc3339_known_dates() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(1_795_219_200 + 3661), "2026-11-21T01:01:01Z");
    }

    async fn roundtrip(url: &str) {
        let s = Store::open(url, 2).await.expect("open");
        for st in ["completed", "error", "timeout"] {
            s.record("async () => 1", &outcome(st))
                .await
                .expect("record");
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        let h = s.history(10).await.expect("history");
        assert_eq!(h.len(), 2, "pruned to keep=2");
        assert_eq!((h[0].tool_calls, h[0].duration_ms), (2, 7));
    }

    #[tokio::test]
    async fn sqlite_roundtrip_and_prune() {
        let dir = tempfile::tempdir().expect("tmp");
        roundtrip(&sqlite_url(&dir.path().join("a b ñ.db"))).await;
    }

    /// Runs only when `AMC_TEST_POSTGRES` is set (CI service container).
    #[tokio::test]
    async fn postgres_roundtrip_and_prune() {
        if let Ok(url) = std::env::var("AMC_TEST_POSTGRES") {
            roundtrip(&url).await;
        }
    }
}
