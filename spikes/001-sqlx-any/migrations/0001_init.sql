CREATE TABLE executions (
    id TEXT PRIMARY KEY,
    ts TEXT NOT NULL,
    code TEXT NOT NULL,
    status TEXT NOT NULL,
    result TEXT,
    duration_ms BIGINT NOT NULL,
    tool_calls BIGINT NOT NULL
);
CREATE INDEX executions_ts ON executions (ts);
