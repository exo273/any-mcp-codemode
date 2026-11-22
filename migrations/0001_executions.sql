-- Portable across SQLite and Postgres (sqlx Any): TEXT/BIGINT only, RFC 3339 timestamps.
CREATE TABLE executions (
    id TEXT PRIMARY KEY,
    ts TEXT NOT NULL,
    code TEXT NOT NULL,
    status TEXT NOT NULL,
    result TEXT,
    error TEXT,
    logs TEXT NOT NULL,
    tool_calls BIGINT NOT NULL,
    duration_ms BIGINT NOT NULL
);
CREATE INDEX executions_ts ON executions (ts);
