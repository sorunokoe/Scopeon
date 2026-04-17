use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use rusqlite_migration::{Migrations, M};
use std::path::{Path, PathBuf};

use crate::cost::{cache_hit_rate, cache_savings_usd};
use crate::models::*;

/// Minimum number of input tokens in the preceding turn for a compaction event to
/// be recognised. Below this threshold a sudden token drop is more likely to be a
/// very short follow-up message than a genuine context compaction.
pub const COMPACTION_MIN_PREV_TOKENS: i64 = 50_000;

/// Fraction by which input tokens must fall relative to the previous turn to be
/// classified as a compaction event (empirically, Claude's compaction reduces
/// context by ~60–80 %; 50 % gives a comfortable margin below genuine drops).
const COMPACTION_DROP_THRESHOLD: f64 = 0.50;

static MIGRATIONS: &[&str] = &[
    // M0001 — initial schema
    "CREATE TABLE IF NOT EXISTS sessions (
        id TEXT PRIMARY KEY,
        project TEXT NOT NULL DEFAULT '',
        project_name TEXT NOT NULL DEFAULT '',
        slug TEXT NOT NULL DEFAULT '',
        model TEXT NOT NULL DEFAULT '',
        git_branch TEXT NOT NULL DEFAULT '',
        started_at INTEGER NOT NULL,
        last_turn_at INTEGER NOT NULL,
        total_turns INTEGER NOT NULL DEFAULT 0,
        is_subagent INTEGER NOT NULL DEFAULT 0,
        parent_session_id TEXT
    );
    CREATE TABLE IF NOT EXISTS turns (
        id TEXT PRIMARY KEY,
        session_id TEXT NOT NULL,
        turn_index INTEGER NOT NULL,
        timestamp INTEGER NOT NULL,
        duration_ms INTEGER,
        input_tokens INTEGER NOT NULL DEFAULT 0,
        cache_read_tokens INTEGER NOT NULL DEFAULT 0,
        cache_write_tokens INTEGER NOT NULL DEFAULT 0,
        cache_write_5m_tokens INTEGER NOT NULL DEFAULT 0,
        cache_write_1h_tokens INTEGER NOT NULL DEFAULT 0,
        output_tokens INTEGER NOT NULL DEFAULT 0,
        thinking_tokens INTEGER NOT NULL DEFAULT 0,
        mcp_call_count INTEGER NOT NULL DEFAULT 0,
        mcp_input_token_est INTEGER NOT NULL DEFAULT 0,
        text_output_tokens INTEGER NOT NULL DEFAULT 0,
        model TEXT NOT NULL DEFAULT '',
        service_tier TEXT NOT NULL DEFAULT '',
        estimated_cost_usd REAL NOT NULL DEFAULT 0.0,
        FOREIGN KEY (session_id) REFERENCES sessions(id)
    );
    CREATE TABLE IF NOT EXISTS tool_calls (
        id TEXT PRIMARY KEY,
        turn_id TEXT NOT NULL,
        session_id TEXT NOT NULL,
        tool_name TEXT NOT NULL DEFAULT '',
        input_size_chars INTEGER NOT NULL DEFAULT 0,
        timestamp INTEGER NOT NULL,
        FOREIGN KEY (turn_id) REFERENCES turns(id)
    );
    CREATE TABLE IF NOT EXISTS daily_rollup (
        date TEXT PRIMARY KEY,
        total_input_tokens INTEGER NOT NULL DEFAULT 0,
        total_cache_read_tokens INTEGER NOT NULL DEFAULT 0,
        total_cache_write_tokens INTEGER NOT NULL DEFAULT 0,
        total_output_tokens INTEGER NOT NULL DEFAULT 0,
        total_thinking_tokens INTEGER NOT NULL DEFAULT 0,
        total_mcp_calls INTEGER NOT NULL DEFAULT 0,
        session_count INTEGER NOT NULL DEFAULT 0,
        turn_count INTEGER NOT NULL DEFAULT 0,
        estimated_cost_usd REAL NOT NULL DEFAULT 0.0
    );
    CREATE TABLE IF NOT EXISTS file_offsets (
        file_path TEXT PRIMARY KEY,
        byte_offset INTEGER NOT NULL DEFAULT 0,
        last_modified INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX IF NOT EXISTS idx_turns_session ON turns(session_id);
    CREATE INDEX IF NOT EXISTS idx_turns_timestamp ON turns(timestamp);
    CREATE INDEX IF NOT EXISTS idx_tool_calls_turn ON tool_calls(turn_id);",
    // M0002 — compaction and anomaly columns
    "ALTER TABLE turns ADD COLUMN is_compaction_event INTEGER NOT NULL DEFAULT 0;
     ALTER TABLE sessions ADD COLUMN had_compaction INTEGER NOT NULL DEFAULT 0;
     ALTER TABLE sessions ADD COLUMN is_expensive INTEGER NOT NULL DEFAULT 0;
     ALTER TABLE sessions ADD COLUMN is_tool_heavy INTEGER NOT NULL DEFAULT 0;
     ALTER TABLE sessions ADD COLUMN is_cache_cold INTEGER NOT NULL DEFAULT 0;",
    // M0003 — performance index on sessions.last_turn_at (for ORDER BY / WHERE queries)
    "CREATE INDEX IF NOT EXISTS idx_sessions_last_turn_at ON sessions(last_turn_at DESC);",
    // M0004 — session tags (user-defined labels for filtering and cost attribution)
    "CREATE TABLE IF NOT EXISTS session_tags (
        session_id TEXT NOT NULL,
        tag TEXT NOT NULL,
        PRIMARY KEY (session_id, tag),
        FOREIGN KEY (session_id) REFERENCES sessions(id)
    );
    CREATE INDEX IF NOT EXISTS idx_session_tags_tag ON session_tags(tag);",
    // M0005 — health score trend storage (TRIZ D6: enables per-day health history sparkline)
    "ALTER TABLE daily_rollup ADD COLUMN health_score_avg REAL NOT NULL DEFAULT 0.0;",
    // M0006 — tool call input hash for accurate redundancy detection
    // (replaces input_size_chars key; old rows default to 0 = fall back to size-based match)
    "ALTER TABLE tool_calls ADD COLUMN input_hash INTEGER NOT NULL DEFAULT 0;",
    // M0007 — §8.2: Store actual context window size from API response per-session.
    // NULL means unknown — callers fall back to model-prefix table in context.rs.
    "ALTER TABLE sessions ADD COLUMN context_window_tokens INTEGER;",
    // M0008 — hot-path composite index for last-turn and recent-turn lookups.
    "CREATE INDEX IF NOT EXISTS idx_turns_session_turn_index ON turns(session_id, turn_index DESC);",
    // M0009 — provider metadata plus normalized provenance/task history tables.
    "ALTER TABLE sessions ADD COLUMN provider TEXT NOT NULL DEFAULT '';
     ALTER TABLE sessions ADD COLUMN provider_version TEXT NOT NULL DEFAULT '';
     CREATE TABLE IF NOT EXISTS interaction_events (
         id TEXT PRIMARY KEY,
         session_id TEXT NOT NULL,
         turn_id TEXT,
         task_run_id TEXT,
         correlation_id TEXT,
         parent_id TEXT,
         provider TEXT NOT NULL DEFAULT '',
         timestamp INTEGER NOT NULL,
         kind TEXT NOT NULL DEFAULT '',
         phase TEXT NOT NULL DEFAULT '',
         name TEXT NOT NULL DEFAULT '',
         display_name TEXT,
         mcp_server TEXT,
         mcp_tool TEXT,
         hook_type TEXT,
         agent_type TEXT,
         execution_mode TEXT,
         model TEXT,
         status TEXT,
         success INTEGER,
         input_size_chars INTEGER NOT NULL DEFAULT 0,
         output_size_chars INTEGER NOT NULL DEFAULT 0,
         prompt_size_chars INTEGER NOT NULL DEFAULT 0,
         summary_size_chars INTEGER NOT NULL DEFAULT 0,
         total_tokens INTEGER,
         total_tool_calls INTEGER,
         duration_ms INTEGER,
         estimated_input_tokens INTEGER NOT NULL DEFAULT 0,
         estimated_output_tokens INTEGER NOT NULL DEFAULT 0,
         estimated_cost_usd REAL NOT NULL DEFAULT 0.0,
         confidence TEXT NOT NULL DEFAULT 'unavailable',
         FOREIGN KEY (session_id) REFERENCES sessions(id)
     );
     CREATE INDEX IF NOT EXISTS idx_interaction_events_session_time
         ON interaction_events(session_id, timestamp ASC);
     CREATE INDEX IF NOT EXISTS idx_interaction_events_kind_time
         ON interaction_events(kind, timestamp DESC);
     CREATE TABLE IF NOT EXISTS task_runs (
         id TEXT PRIMARY KEY,
         session_id TEXT NOT NULL,
         correlation_id TEXT,
         name TEXT NOT NULL DEFAULT '',
         display_name TEXT,
         agent_type TEXT NOT NULL DEFAULT '',
         execution_mode TEXT NOT NULL DEFAULT '',
         requested_model TEXT,
         actual_model TEXT,
         started_at INTEGER NOT NULL,
         completed_at INTEGER,
         duration_ms INTEGER,
         success INTEGER,
         total_tokens INTEGER,
         total_tool_calls INTEGER,
         description_size_chars INTEGER NOT NULL DEFAULT 0,
         prompt_size_chars INTEGER NOT NULL DEFAULT 0,
         summary_size_chars INTEGER NOT NULL DEFAULT 0,
         confidence TEXT NOT NULL DEFAULT 'unavailable',
         FOREIGN KEY (session_id) REFERENCES sessions(id)
     );
     CREATE INDEX IF NOT EXISTS idx_task_runs_session_time
         ON task_runs(session_id, started_at DESC);",
];

/// Handle to the Scopeon SQLite database.
pub struct Database {
    conn: Connection,
    /// Filesystem path of the database file, if opened from disk.
    /// `None` for in-memory databases (tests).
    db_path: Option<PathBuf>,
}

impl Database {
    /// Open (or create) the database at `path`. Runs migrations automatically.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("Opening database at {}", path.display()))?;

        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;",
        )?;

        let mut conn = conn;
        let migrations = Migrations::new(MIGRATIONS.iter().map(|sql| M::up(sql)).collect());
        migrations
            .to_latest(&mut conn)
            .context("Running database migrations")?;

        Ok(Database {
            conn,
            db_path: Some(path.to_owned()),
        })
    }

    /// Open an in-memory database. Used in tests and benchmarks.
    ///
    /// Not gated by `#[cfg(test)]` so integration tests in other crates can use it.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")?;
        let mut conn = conn;
        let migrations = Migrations::new(MIGRATIONS.iter().map(|sql| M::up(sql)).collect());
        migrations
            .to_latest(&mut conn)
            .context("Running migrations on in-memory DB")?;
        Ok(Database {
            conn,
            db_path: None,
        })
    }

    /// Open a read-only connection to an existing on-disk database.
    ///
    /// Uses `PRAGMA query_only=ON` to prevent accidental writes. WAL mode lets
    /// multiple read connections coexist with one write connection without any
    /// mutual exclusion, so this is safe to call from the MCP snapshot task even
    /// while the watcher is writing.
    ///
    /// Returns `None` if `db_path` is `None` (i.e., in-memory DB — tests don't
    /// need a separate read connection).
    pub fn open_readonly(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("Opening read-only DB at {}", path.display()))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA query_only=ON; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;",
        )?;
        Ok(Database {
            conn,
            db_path: Some(path.to_owned()),
        })
    }

    /// Return the filesystem path this database was opened from, or `None` for
    /// in-memory databases.
    pub fn path(&self) -> Option<&Path> {
        self.db_path.as_deref()
    }

    // ── File offset tracking (for incremental parsing) ──────────────────────

    pub fn get_file_offset(&self, file_path: &str) -> Result<u64> {
        let offset: Option<i64> = self
            .conn
            .query_row(
                "SELECT byte_offset FROM file_offsets WHERE file_path = ?1",
                params![file_path],
                |row| row.get(0),
            )
            .ok();
        Ok(offset.unwrap_or(0).max(0) as u64)
    }

    /// Returns `(byte_offset, last_modified_ms)` for the given file path.
    /// Returns `(0, 0)` if the path has never been recorded.
    pub fn get_file_offset_and_mtime(&self, file_path: &str) -> Result<(u64, i64)> {
        let row: Option<(i64, i64)> = self
            .conn
            .query_row(
                "SELECT byte_offset, last_modified FROM file_offsets WHERE file_path = ?1",
                params![file_path],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();
        match row {
            Some((offset, mtime)) => Ok((offset.max(0) as u64, mtime)),
            None => Ok((0, 0)),
        }
    }

    pub fn set_file_offset(
        &self,
        file_path: &str,
        byte_offset: u64,
        last_modified: i64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO file_offsets (file_path, byte_offset, last_modified) VALUES (?1, ?2, ?3)",
            params![file_path, byte_offset as i64, last_modified],
        )?;
        Ok(())
    }

    pub fn get_all_file_offsets(&self) -> Result<Vec<(String, u64)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT file_path, byte_offset FROM file_offsets")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    // ── Session upsert ───────────────────────────────────────────────────────

    pub fn upsert_session(&self, session: &Session) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sessions
                (id, project, project_name, slug, provider, provider_version, model, git_branch, started_at, last_turn_at, total_turns, is_subagent, parent_session_id, context_window_tokens)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
             ON CONFLICT(id) DO UPDATE SET
                provider = COALESCE(NULLIF(excluded.provider, ''), sessions.provider),
                provider_version = COALESCE(NULLIF(excluded.provider_version, ''), sessions.provider_version),
                model = excluded.model,
                last_turn_at = excluded.last_turn_at,
                total_turns = excluded.total_turns,
                git_branch = excluded.git_branch,
                is_subagent = excluded.is_subagent,
                parent_session_id = excluded.parent_session_id,
                context_window_tokens = COALESCE(excluded.context_window_tokens, sessions.context_window_tokens)",
            params![
                session.id,
                session.project,
                session.project_name,
                session.slug,
                session.provider,
                session.provider_version,
                session.model,
                session.git_branch,
                session.started_at,
                session.last_turn_at,
                session.total_turns,
                session.is_subagent as i32,
                session.parent_session_id,
                session.context_window_tokens,
            ],
        )?;
        Ok(())
    }

    // ── Turn upsert ──────────────────────────────────────────────────────────

    pub fn upsert_turn(&self, turn: &Turn) -> Result<()> {
        self.conn.execute(
            "INSERT INTO turns
                (id, session_id, turn_index, timestamp, duration_ms,
                 input_tokens, cache_read_tokens, cache_write_tokens,
                 cache_write_5m_tokens, cache_write_1h_tokens, output_tokens,
                 thinking_tokens, mcp_call_count, mcp_input_token_est, text_output_tokens,
                 model, service_tier, estimated_cost_usd)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)
             ON CONFLICT(id) DO UPDATE SET
                 turn_index         = excluded.turn_index,
                 timestamp          = excluded.timestamp,
                 duration_ms        = excluded.duration_ms,
                 input_tokens       = excluded.input_tokens,
                 cache_read_tokens  = excluded.cache_read_tokens,
                 cache_write_tokens = excluded.cache_write_tokens,
                 cache_write_5m_tokens = excluded.cache_write_5m_tokens,
                 cache_write_1h_tokens = excluded.cache_write_1h_tokens,
                 output_tokens      = excluded.output_tokens,
                 thinking_tokens    = excluded.thinking_tokens,
                 mcp_call_count     = excluded.mcp_call_count,
                 mcp_input_token_est = excluded.mcp_input_token_est,
                 text_output_tokens = excluded.text_output_tokens,
                 model              = excluded.model,
                 service_tier       = excluded.service_tier,
                 estimated_cost_usd = excluded.estimated_cost_usd
                 -- is_compaction_event intentionally omitted: once set by the
                 -- compaction detector it must not be overwritten by a re-parse.",
            params![
                turn.id,
                turn.session_id,
                turn.turn_index,
                turn.timestamp,
                turn.duration_ms,
                turn.input_tokens,
                turn.cache_read_tokens,
                turn.cache_write_tokens,
                turn.cache_write_5m_tokens,
                turn.cache_write_1h_tokens,
                turn.output_tokens,
                turn.thinking_tokens,
                turn.mcp_call_count,
                turn.mcp_input_token_est,
                turn.text_output_tokens,
                turn.model,
                turn.service_tier,
                turn.estimated_cost_usd,
            ],
        )?;
        Ok(())
    }

    pub fn upsert_tool_call(&self, tc: &ToolCall) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO tool_calls (id, turn_id, session_id, tool_name, input_size_chars, input_hash, timestamp)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![tc.id, tc.turn_id, tc.session_id, tc.tool_name, tc.input_size_chars, tc.input_hash as i64, tc.timestamp],
        )?;
        Ok(())
    }

    // ── Queries ──────────────────────────────────────────────────────────────

    pub fn get_session(&self, session_id: &str) -> Result<Option<Session>> {
        let result = self.conn.query_row(
            "SELECT id, project, project_name, slug, provider, provider_version, model,
                    git_branch, started_at, last_turn_at, total_turns, is_subagent,
                    parent_session_id, context_window_tokens
             FROM sessions WHERE id = ?1",
            params![session_id],
            row_to_session,
        );
        match result {
            Ok(s) => Ok(Some(s)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn list_sessions(&self, limit: usize) -> Result<Vec<Session>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project, project_name, slug, provider, provider_version, model,
                    git_branch, started_at, last_turn_at, total_turns, is_subagent,
                    parent_session_id, context_window_tokens
             FROM sessions
             ORDER BY last_turn_at DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], row_to_session)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Fetch cost + cache hit rate for all sessions in a single GROUP BY query.
    /// Returns a map from session_id → SessionSummary for O(1) lookup in the list view.
    pub fn list_session_summaries(&self, limit: usize) -> Result<Vec<SessionSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_id,
                    SUM(estimated_cost_usd) AS cost,
                    COALESCE(SUM(input_tokens), 0) AS total_input,
                    COALESCE(SUM(cache_read_tokens), 0) AS cr,
                    COALESCE(SUM(cache_write_tokens), 0) AS cw
             FROM turns
             WHERE session_id IN (
                 SELECT id FROM sessions ORDER BY last_turn_at DESC LIMIT ?1
             )
             GROUP BY session_id",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            let session_id: String = row.get(0)?;
            let cost: f64 = row.get(1)?;
            let total_input: i64 = row.get(2)?;
            let cache_read: i64 = row.get(3)?;
            let cache_write: i64 = row.get(4)?;
            let hit_rate = cache_hit_rate(total_input, cache_read, cache_write);
            Ok(SessionSummary {
                session_id,
                estimated_cost_usd: cost,
                cache_hit_rate: hit_rate,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn list_turns_for_session(&self, session_id: &str) -> Result<Vec<Turn>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, turn_index, timestamp, duration_ms,
                    input_tokens, cache_read_tokens, cache_write_tokens,
                    cache_write_5m_tokens, cache_write_1h_tokens, output_tokens,
                    thinking_tokens, mcp_call_count, mcp_input_token_est, text_output_tokens,
                    model, service_tier, estimated_cost_usd, is_compaction_event
             FROM turns WHERE session_id = ?1 ORDER BY turn_index ASC",
        )?;
        let rows = stmt.query_map(params![session_id], row_to_turn)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// §1.1: Compute session aggregates using a single SQL SUM query instead of
    /// deserializing all Turn rows into memory and iterating in Rust.
    /// Returns a SessionStats with aggregate fields populated and `turns` empty.
    /// Use this for API/metric paths. Use `get_session_stats` only when per-turn
    /// data is actually needed (e.g., MCP session history, per-turn analysis).
    pub fn get_session_aggregates(&self, session_id: &str) -> Result<SessionStats> {
        let session = self.get_session(session_id)?;
        let row = self.conn.query_row(
            "SELECT
                 COUNT(*),
                 COALESCE(SUM(input_tokens), 0),
                 COALESCE(SUM(cache_read_tokens), 0),
                 COALESCE(SUM(cache_write_tokens), 0),
                 COALESCE(SUM(output_tokens), 0),
                 COALESCE(SUM(thinking_tokens), 0),
                 COALESCE(SUM(mcp_call_count), 0),
                 COALESCE(SUM(estimated_cost_usd), 0.0)
             FROM turns WHERE session_id = ?1",
            params![session_id],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, i64>(6)?,
                    r.get::<_, f64>(7)?,
                ))
            },
        )?;
        let (n_turns, input, cache_read, cache_write, output, thinking, mcp, cost) = row;
        let cache_sav = cache_savings_usd("", cache_read, cache_write); // model-agnostic estimate
        let hit_rate = cache_hit_rate(input, cache_read, cache_write);
        Ok(SessionStats {
            session,
            total_turns: n_turns,
            total_input_tokens: input,
            total_cache_read_tokens: cache_read,
            total_cache_write_tokens: cache_write,
            total_output_tokens: output,
            total_thinking_tokens: thinking,
            total_mcp_calls: mcp,
            estimated_cost_usd: cost,
            cache_savings_usd: cache_sav,
            cache_hit_rate: hit_rate,
            turns: vec![],
        })
    }

    /// Return only the last turn for a session — O(1) memory, no full scan.
    /// Use this for context-pressure calculations that need only the most recent tokens.
    pub fn get_last_turn_for_session(&self, session_id: &str) -> Result<Option<Turn>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, turn_index, timestamp, duration_ms,
                    input_tokens, cache_read_tokens, cache_write_tokens,
                    cache_write_5m_tokens, cache_write_1h_tokens,
                    output_tokens, thinking_tokens, mcp_call_count,
                    mcp_input_token_est, text_output_tokens, model, service_tier,
                    estimated_cost_usd, is_compaction_event
             FROM turns WHERE session_id = ?1 ORDER BY turn_index DESC LIMIT 1",
        )?;
        let mut rows = stmt.query_map(params![session_id], row_to_turn)?;
        match rows.next() {
            Some(Ok(t)) => Ok(Some(t)),
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
    }

    pub fn get_session_stats(&self, session_id: &str) -> Result<SessionStats> {
        let session = self.get_session(session_id)?;
        let turns = self.list_turns_for_session(session_id)?;

        let mut stats = SessionStats {
            session,
            ..Default::default()
        };

        for t in &turns {
            stats.total_input_tokens += t.input_tokens;
            stats.total_cache_read_tokens += t.cache_read_tokens;
            stats.total_cache_write_tokens += t.cache_write_tokens;
            stats.total_output_tokens += t.output_tokens;
            stats.total_thinking_tokens += t.thinking_tokens;
            stats.total_mcp_calls += t.mcp_call_count;
            stats.estimated_cost_usd += t.estimated_cost_usd;
            stats.cache_savings_usd +=
                cache_savings_usd(&t.model, t.cache_read_tokens, t.cache_write_tokens);
        }
        stats.total_turns = turns.len() as i64;

        let total_input_billable = stats.total_input_tokens
            + stats.total_cache_read_tokens
            + stats.total_cache_write_tokens;
        if total_input_billable > 0 {
            stats.cache_hit_rate = cache_hit_rate(
                stats.total_input_tokens,
                stats.total_cache_read_tokens,
                stats.total_cache_write_tokens,
            );
        }

        stats.turns = turns;
        Ok(stats)
    }

    pub fn get_latest_session_id(&self) -> Result<Option<String>> {
        let result = self.conn.query_row(
            "SELECT id FROM sessions ORDER BY last_turn_at DESC LIMIT 1",
            [],
            |row| row.get(0),
        );
        match result {
            Ok(id) => Ok(Some(id)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn get_daily_rollups(&self, days: i64) -> Result<Vec<DailyRollup>> {
        let mut stmt = self.conn.prepare(
            "SELECT date, total_input_tokens, total_cache_read_tokens, total_cache_write_tokens,
                    total_output_tokens, total_thinking_tokens, total_mcp_calls,
                    session_count, turn_count, estimated_cost_usd, health_score_avg
             FROM daily_rollup
             ORDER BY date DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![days], row_to_daily)?;
        let mut v: Vec<DailyRollup> = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        v.reverse(); // chronological order
        Ok(v)
    }

    /// Compute `(cache_hit_rate, thinking_ratio, avg_input_per_turn)` tuples from
    /// the last 90 days of daily rollup data, for use by the adaptive threshold engine.
    ///
    /// Only days with at least 100 total input tokens are included to avoid
    /// noise from near-empty days skewing percentiles.
    pub fn get_threshold_data(&self) -> Result<Vec<(f64, f64, f64)>> {
        let rollups = self.get_daily_rollups(90)?;
        let data = rollups
            .iter()
            .filter(|r| r.total_input_tokens + r.total_cache_read_tokens > 100)
            .map(|r| {
                let total = (r.total_input_tokens
                    + r.total_cache_read_tokens
                    + r.total_cache_write_tokens) as f64;
                let cache_rate = if total > 0.0 {
                    r.total_cache_read_tokens as f64 / total
                } else {
                    0.0
                };
                let thinking_ratio = if r.total_output_tokens > 0 {
                    r.total_thinking_tokens as f64 / r.total_output_tokens as f64
                } else {
                    0.0
                };
                let avg_input = if r.turn_count > 0 {
                    r.total_input_tokens as f64 / r.turn_count as f64
                } else {
                    0.0
                };
                (cache_rate, thinking_ratio, avg_input)
            })
            .collect();
        Ok(data)
    }

    pub fn refresh_daily_rollup(&self) -> Result<()> {
        // Full recompute — used only for reprice and initial backfill.
        // For incremental updates after file events, use refresh_daily_rollup_for_timestamps.
        // health_score_avg defaults to 0.0; read paths must not mutate it.
        self.conn.execute_batch(
            "BEGIN;
             DELETE FROM daily_rollup;
             INSERT INTO daily_rollup
             SELECT
                date(timestamp / 1000, 'unixepoch', 'localtime') AS date,
                SUM(input_tokens),
                SUM(cache_read_tokens),
                SUM(cache_write_tokens),
                SUM(output_tokens),
                SUM(thinking_tokens),
                SUM(mcp_call_count),
                COUNT(DISTINCT session_id),
                COUNT(*),
                SUM(estimated_cost_usd),
                0.0
             FROM turns
             GROUP BY date
             ORDER BY date;
             COMMIT;",
        )?;
        Ok(())
    }

    /// Incremental rollup: recompute only the calendar dates that appear in `timestamps_ms`.
    /// Call this after inserting a batch of turns to avoid a full-table recompute.
    pub fn refresh_daily_rollup_for_timestamps(&self, timestamps_ms: &[i64]) -> Result<()> {
        if timestamps_ms.is_empty() {
            return Ok(());
        }
        // Collect the unique dates affected by this batch.
        let dates: Vec<String> = {
            let mut stmt = self.conn.prepare(
                "SELECT DISTINCT date(timestamp / 1000, 'unixepoch', 'localtime')
                 FROM turns
                 WHERE timestamp IN (SELECT value FROM json_each(?))",
            )?;
            // Build a JSON array of timestamp values for the IN clause.
            let json_arr = serde_json::to_string(timestamps_ms)?;
            let rows = stmt.query_map([json_arr], |r| r.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        // §1.4: Wrap all per-date upserts in a single explicit transaction to avoid
        // one implicit fsync per date (common on first backfill with many dates).
        let tx = self.conn.unchecked_transaction()?;
        {
            // Upsert each affected date only.
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO daily_rollup
                 SELECT
                    date(timestamp / 1000, 'unixepoch', 'localtime') AS date,
                    SUM(input_tokens),
                    SUM(cache_read_tokens),
                    SUM(cache_write_tokens),
                    SUM(output_tokens),
                    SUM(thinking_tokens),
                    SUM(mcp_call_count),
                    COUNT(DISTINCT session_id),
                    COUNT(*),
                    SUM(estimated_cost_usd),
                    COALESCE(
                        (SELECT health_score_avg FROM daily_rollup WHERE date = date(turns.timestamp / 1000, 'unixepoch', 'localtime')),
                        0.0
                    )
                 FROM turns
                 WHERE date(timestamp / 1000, 'unixepoch', 'localtime') = ?
                 GROUP BY date",
            )?;
            for date in &dates {
                stmt.execute([date])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// IS-M: Get cache efficiency (cache_read / (input + cache_read + cache_write)) for the last N turns
    /// of a session, ordered oldest-first. Used to detect cache bust events.
    pub fn get_cache_efficiency_trend(&self, session_id: &str, last_n: usize) -> Result<Vec<f64>> {
        let mut stmt = self.conn.prepare(
            "SELECT input_tokens, cache_read_tokens, cache_write_tokens
             FROM turns
             WHERE session_id = ?1
             ORDER BY turn_index DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![session_id, last_n as i64], |row| {
            let input: i64 = row.get(0)?;
            let cache_read: i64 = row.get(1)?;
            let cache_write: i64 = row.get(2)?;
            Ok((input, cache_read, cache_write))
        })?;
        let mut efficiencies: Vec<f64> = rows
            .filter_map(|r| r.ok())
            .map(|(input, cache_read, cache_write)| {
                let denom = input + cache_read + cache_write;
                if denom > 0 {
                    cache_read as f64 / denom as f64
                } else {
                    0.0
                }
            })
            .collect();
        // Return in chronological order (oldest first).
        efficiencies.reverse();
        Ok(efficiencies)
    }

    /// IS-E: Compute the 90-day median tokens per turn as a Bayesian prior
    /// for the context countdown cold-start (turns 0-2 where no session data exists).
    /// §5.3: Renamed from `get_median_tokens_per_turn` — the SQL uses AVG (arithmetic mean),
    /// not a median. Callers rely on this for the Bayesian cold-start prior (IS-E).
    pub fn get_mean_tokens_per_turn(&self, days: u32) -> Result<f64> {
        let cutoff_ms =
            chrono::Utc::now().timestamp_millis() - (days as i64) * 24 * 60 * 60 * 1_000;
        let mut stmt = self.conn.prepare(
            "SELECT CAST(SUM(input_tokens) AS REAL) / COUNT(*)
             FROM turns
             WHERE timestamp > ?1 AND input_tokens > 0",
        )?;
        let mean: Option<f64> = stmt.query_row(params![cutoff_ms], |row| row.get(0)).ok();
        Ok(mean.unwrap_or(50_000.0)) // 50k default for new users
    }

    /// IS-K: Get total cost grouped by session tag for the last `days` days.
    /// Queries the `session_tags` table (multi-tag system used by `scopeon tag set`).
    /// Returns `(tag, total_cost_usd, session_count)`.
    pub fn get_cost_by_tag_days(&self, days: u32) -> Result<Vec<(String, f64, i64)>> {
        let cutoff_ms =
            chrono::Utc::now().timestamp_millis() - (days as i64) * 24 * 60 * 60 * 1_000;
        let mut stmt = self.conn.prepare(
            "SELECT st.tag,
                    SUM(t.estimated_cost_usd) AS total_cost,
                    COUNT(DISTINCT st.session_id) AS session_count
             FROM session_tags st
             JOIN turns t ON t.session_id = st.session_id
             WHERE t.timestamp > ?1
             GROUP BY st.tag
             ORDER BY total_cost DESC",
        )?;
        let rows = stmt.query_map(params![cutoff_ms], |row| {
            let tag: String = row.get(0)?;
            let cost: f64 = row.get(1)?;
            let count: i64 = row.get(2)?;
            Ok((tag, cost, count))
        })?;
        rows.map(|r| r.map_err(Into::into)).collect()
    }

    /// Delete turns (and orphaned sessions) older than `days` days.
    ///
    /// TRIZ D5: Resolves NE-E (unbounded DB growth). Opt-in via `[storage] retain_days`.
    /// Returns the number of turns deleted.
    pub fn delete_turns_older_than(&self, days: u64) -> Result<usize> {
        let cutoff_ms =
            chrono::Utc::now().timestamp_millis() - (days as i64) * 24 * 60 * 60 * 1_000;
        // §3.4: All three DML statements must be atomic — a crash between DELETE tool_calls
        // and DELETE turns leaves orphaned tool_calls rows. Wrap in an explicit transaction.
        let tx = self.conn.unchecked_transaction()?;
        // Delete tool_calls first to satisfy the FK constraint (no CASCADE defined).
        tx.execute(
            "DELETE FROM tool_calls WHERE turn_id IN \
             (SELECT id FROM turns WHERE timestamp < ?1)",
            params![cutoff_ms],
        )?;
        let deleted = tx.execute("DELETE FROM turns WHERE timestamp < ?1", params![cutoff_ms])?;
        tx.execute(
            "DELETE FROM interaction_events
             WHERE session_id NOT IN (SELECT DISTINCT session_id FROM turns)",
            [],
        )?;
        tx.execute(
            "DELETE FROM task_runs
             WHERE session_id NOT IN (SELECT DISTINCT session_id FROM turns)",
            [],
        )?;
        tx.execute(
            "DELETE FROM session_tags
             WHERE session_id NOT IN (SELECT DISTINCT session_id FROM turns)",
            [],
        )?;
        // Remove sessions that have no remaining turns.
        tx.execute(
            "DELETE FROM sessions WHERE id NOT IN (SELECT DISTINCT session_id FROM turns)",
            [],
        )?;
        tx.commit()?;
        // Refresh daily rollup so the UI reflects the purge.
        self.refresh_daily_rollup()?;
        Ok(deleted)
    }

    /// Reprice all turns in a single transaction and refresh the daily rollup.
    /// Returns the number of turns updated and the total cost delta.
    pub fn reprice_all_in_transaction<F>(&self, compute_cost: F) -> Result<(usize, usize, f64)>
    where
        F: Fn(&Turn) -> f64,
    {
        let turns = self.list_all_turns_for_reprice()?;
        let total = turns.len();
        let mut updated = 0usize;
        let mut cost_delta = 0.0f64;
        {
            // SAFETY: `unchecked_transaction` is used instead of `transaction` because
            // `self.conn` is behind `&self` (not `&mut self`). Nested transactions are
            // impossible here: `Database` is always accessed through `Arc<Mutex<Database>>`,
            // so at most one caller holds the mutex at a time, and `reprice_all_in_transaction`
            // is not recursive. The borrow-checker guarantee is upheld informally by the mutex.
            let tx = self.conn.unchecked_transaction()?;
            for turn in &turns {
                let new_cost = compute_cost(turn);
                let delta = new_cost - turn.estimated_cost_usd;
                if delta.abs() > 1e-9 {
                    tx.execute(
                        "UPDATE turns SET estimated_cost_usd = ?1 WHERE id = ?2",
                        params![new_cost, turn.id],
                    )?;
                    cost_delta += delta;
                    updated += 1;
                }
            }
            tx.commit()?;
        }
        self.refresh_daily_rollup()?;
        Ok((updated, total, cost_delta))
    }

    pub fn get_global_stats(&self) -> Result<GlobalStats> {
        let row: (i64, i64, i64, i64, i64, i64, i64, i64, f64) = self.conn.query_row(
            "SELECT
                COUNT(DISTINCT session_id),
                COUNT(*),
                COALESCE(SUM(input_tokens), 0),
                COALESCE(SUM(cache_read_tokens), 0),
                COALESCE(SUM(cache_write_tokens), 0),
                COALESCE(SUM(output_tokens), 0),
                COALESCE(SUM(thinking_tokens), 0),
                COALESCE(SUM(mcp_call_count), 0),
                COALESCE(SUM(estimated_cost_usd), 0.0)
             FROM turns",
            [],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                ))
            },
        )?;

        // cache_hit_rate: read / (input + read + write) — canonical formula from cost.rs
        let cache_hit_rate_val = cache_hit_rate(row.2, row.3, row.4);

        // Compute cache net savings: (read saving) - (write overhead), grouped by model.
        let cache_savings_usd = {
            let mut stmt = self.conn.prepare(
                "SELECT model, COALESCE(SUM(cache_read_tokens), 0), COALESCE(SUM(cache_write_tokens), 0)
                 FROM turns GROUP BY model",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })?;
            rows.flatten()
                .map(|(model, read_tok, write_tok)| cache_savings_usd(&model, read_tok, write_tok))
                .sum::<f64>()
        };

        let daily = self.get_daily_rollups(90)?;

        Ok(GlobalStats {
            total_sessions: row.0,
            total_turns: row.1,
            total_input_tokens: row.2,
            total_cache_read_tokens: row.3,
            total_cache_write_tokens: row.4,
            total_output_tokens: row.5,
            total_thinking_tokens: row.6,
            total_mcp_calls: row.7,
            estimated_cost_usd: row.8,
            cache_savings_usd,
            cache_hit_rate: cache_hit_rate_val,
            daily,
        })
    }

    pub fn count_turns_for_session(&self, session_id: &str) -> Result<i64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM turns WHERE session_id = ?1",
            params![session_id],
            |r| r.get(0),
        )?;
        Ok(count)
    }

    pub fn mark_turn_compaction(&self, turn_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE turns SET is_compaction_event = 1 WHERE id = ?1",
            params![turn_id],
        )?;
        Ok(())
    }

    pub fn mark_session_had_compaction(&self, session_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET had_compaction = 1 WHERE id = ?1",
            params![session_id],
        )?;
        Ok(())
    }

    /// Check whether a newly-inserted turn represents a compaction event by comparing
    /// its input token count against the previous turn in the same session.
    ///
    /// A compaction is detected when:
    /// - The previous turn had > 50 000 input tokens (avoids false positives).
    /// - Input tokens dropped by more than 50% relative to the previous turn.
    ///
    /// Returns `true` if a compaction was detected (and the turn was marked).
    pub fn check_compaction_at_turn(
        &self,
        turn_id: &str,
        session_id: &str,
        turn_index: i64,
        current_input: i64,
    ) -> Result<bool> {
        let prev_input: Option<i64> = self
            .conn
            .query_row(
                "SELECT input_tokens FROM turns
                 WHERE session_id = ?1 AND turn_index < ?2
                 ORDER BY turn_index DESC
                 LIMIT 1",
                params![session_id, turn_index],
                |r| r.get(0),
            )
            .ok();
        if let Some(prev) = prev_input {
            let drop = prev - current_input;
            if prev > COMPACTION_MIN_PREV_TOKENS
                && drop > 0
                && (drop as f64 / prev as f64) > COMPACTION_DROP_THRESHOLD
            {
                self.mark_turn_compaction(turn_id)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Returns the `input_tokens` of the turn immediately before `turn_index`
    /// in the given session, or `None` if no prior turn exists.
    pub fn get_turn_input_before(&self, session_id: &str, turn_index: i64) -> Result<Option<i64>> {
        let val: Option<i64> = self
            .conn
            .query_row(
                "SELECT input_tokens FROM turns
                 WHERE session_id = ?1 AND turn_index < ?2
                 ORDER BY turn_index DESC LIMIT 1",
                params![session_id, turn_index],
                |r| r.get(0),
            )
            .ok();
        Ok(val)
    }

    pub fn list_tool_calls_for_session(&self, session_id: &str) -> Result<Vec<ToolCall>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, turn_id, session_id, tool_name, input_size_chars, input_hash, timestamp
             FROM tool_calls WHERE session_id = ?1 ORDER BY timestamp ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |row| {
            Ok(ToolCall {
                id: row.get(0)?,
                turn_id: row.get(1)?,
                session_id: row.get(2)?,
                tool_name: row.get(3)?,
                input_size_chars: row.get(4)?,
                input_hash: row.get::<_, i64>(5)? as u64,
                timestamp: row.get(6)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn upsert_interaction_event(&self, event: &InteractionEvent) -> Result<()> {
        self.conn.execute(
            "INSERT INTO interaction_events (
                id, session_id, turn_id, task_run_id, correlation_id, parent_id, provider,
                timestamp, kind, phase, name, display_name, mcp_server, mcp_tool, hook_type,
                agent_type, execution_mode, model, status, success, input_size_chars,
                output_size_chars, prompt_size_chars, summary_size_chars, total_tokens,
                total_tool_calls, duration_ms, estimated_input_tokens, estimated_output_tokens,
                estimated_cost_usd, confidence
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31
             )
             ON CONFLICT(id) DO UPDATE SET
                turn_id = COALESCE(excluded.turn_id, interaction_events.turn_id),
                task_run_id = COALESCE(excluded.task_run_id, interaction_events.task_run_id),
                correlation_id = COALESCE(excluded.correlation_id, interaction_events.correlation_id),
                parent_id = COALESCE(excluded.parent_id, interaction_events.parent_id),
                provider = COALESCE(NULLIF(excluded.provider, ''), interaction_events.provider),
                timestamp = excluded.timestamp,
                kind = excluded.kind,
                phase = excluded.phase,
                name = excluded.name,
                display_name = COALESCE(excluded.display_name, interaction_events.display_name),
                mcp_server = COALESCE(excluded.mcp_server, interaction_events.mcp_server),
                mcp_tool = COALESCE(excluded.mcp_tool, interaction_events.mcp_tool),
                hook_type = COALESCE(excluded.hook_type, interaction_events.hook_type),
                agent_type = COALESCE(excluded.agent_type, interaction_events.agent_type),
                execution_mode = COALESCE(excluded.execution_mode, interaction_events.execution_mode),
                model = COALESCE(excluded.model, interaction_events.model),
                status = COALESCE(excluded.status, interaction_events.status),
                success = COALESCE(excluded.success, interaction_events.success),
                input_size_chars = excluded.input_size_chars,
                output_size_chars = excluded.output_size_chars,
                prompt_size_chars = excluded.prompt_size_chars,
                summary_size_chars = excluded.summary_size_chars,
                total_tokens = COALESCE(excluded.total_tokens, interaction_events.total_tokens),
                total_tool_calls = COALESCE(excluded.total_tool_calls, interaction_events.total_tool_calls),
                duration_ms = COALESCE(excluded.duration_ms, interaction_events.duration_ms),
                estimated_input_tokens = excluded.estimated_input_tokens,
                estimated_output_tokens = excluded.estimated_output_tokens,
                estimated_cost_usd = excluded.estimated_cost_usd,
                confidence = excluded.confidence",
            params![
                event.id,
                event.session_id,
                event.turn_id,
                event.task_run_id,
                event.correlation_id,
                event.parent_id,
                event.provider,
                event.timestamp,
                event.kind,
                event.phase,
                event.name,
                event.display_name,
                event.mcp_server,
                event.mcp_tool,
                event.hook_type,
                event.agent_type,
                event.execution_mode,
                event.model,
                event.status,
                event.success.map(|v| v as i32),
                event.input_size_chars,
                event.output_size_chars,
                event.prompt_size_chars,
                event.summary_size_chars,
                event.total_tokens,
                event.total_tool_calls,
                event.duration_ms,
                event.estimated_input_tokens,
                event.estimated_output_tokens,
                event.estimated_cost_usd,
                event.confidence,
            ],
        )?;
        Ok(())
    }

    pub fn list_interaction_events_for_session(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<InteractionEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, turn_id, task_run_id, correlation_id, parent_id, provider,
                    timestamp, kind, phase, name, display_name, mcp_server, mcp_tool, hook_type,
                    agent_type, execution_mode, model, status, success, input_size_chars,
                    output_size_chars, prompt_size_chars, summary_size_chars, total_tokens,
                    total_tool_calls, duration_ms, estimated_input_tokens, estimated_output_tokens,
                    estimated_cost_usd, confidence
              FROM interaction_events
              WHERE session_id = ?1
              ORDER BY timestamp DESC
              LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![session_id, limit as i64], row_to_interaction_event)?;
        let mut events = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        events.reverse();
        Ok(events)
    }

    pub fn list_recent_interaction_events(&self, limit: usize) -> Result<Vec<InteractionEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, turn_id, task_run_id, correlation_id, parent_id, provider,
                    timestamp, kind, phase, name, display_name, mcp_server, mcp_tool, hook_type,
                    agent_type, execution_mode, model, status, success, input_size_chars,
                    output_size_chars, prompt_size_chars, summary_size_chars, total_tokens,
                    total_tool_calls, duration_ms, estimated_input_tokens, estimated_output_tokens,
                    estimated_cost_usd, confidence
             FROM interaction_events
             ORDER BY timestamp DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], row_to_interaction_event)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn upsert_task_run(&self, task: &TaskRun) -> Result<()> {
        self.conn.execute(
            "INSERT INTO task_runs (
                id, session_id, correlation_id, name, display_name, agent_type, execution_mode,
                requested_model, actual_model, started_at, completed_at, duration_ms, success,
                total_tokens, total_tool_calls, description_size_chars, prompt_size_chars,
                summary_size_chars, confidence
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                ?18, ?19
             )
             ON CONFLICT(id) DO UPDATE SET
                correlation_id = COALESCE(excluded.correlation_id, task_runs.correlation_id),
                name = excluded.name,
                display_name = COALESCE(excluded.display_name, task_runs.display_name),
                agent_type = COALESCE(NULLIF(excluded.agent_type, ''), task_runs.agent_type),
                execution_mode = COALESCE(NULLIF(excluded.execution_mode, ''), task_runs.execution_mode),
                requested_model = COALESCE(excluded.requested_model, task_runs.requested_model),
                actual_model = COALESCE(excluded.actual_model, task_runs.actual_model),
                started_at = excluded.started_at,
                completed_at = COALESCE(excluded.completed_at, task_runs.completed_at),
                duration_ms = COALESCE(excluded.duration_ms, task_runs.duration_ms),
                success = COALESCE(excluded.success, task_runs.success),
                total_tokens = COALESCE(excluded.total_tokens, task_runs.total_tokens),
                total_tool_calls = COALESCE(excluded.total_tool_calls, task_runs.total_tool_calls),
                description_size_chars = excluded.description_size_chars,
                prompt_size_chars = excluded.prompt_size_chars,
                summary_size_chars = excluded.summary_size_chars,
                confidence = excluded.confidence",
            params![
                task.id,
                task.session_id,
                task.correlation_id,
                task.name,
                task.display_name,
                task.agent_type,
                task.execution_mode,
                task.requested_model,
                task.actual_model,
                task.started_at,
                task.completed_at,
                task.duration_ms,
                task.success.map(|v| v as i32),
                task.total_tokens,
                task.total_tool_calls,
                task.description_size_chars,
                task.prompt_size_chars,
                task.summary_size_chars,
                task.confidence,
            ],
        )?;
        Ok(())
    }

    pub fn list_task_runs_for_session(&self, session_id: &str) -> Result<Vec<TaskRun>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, correlation_id, name, display_name, agent_type,
                    execution_mode, requested_model, actual_model, started_at, completed_at,
                    duration_ms, success, total_tokens, total_tool_calls,
                    description_size_chars, prompt_size_chars, summary_size_chars, confidence
             FROM task_runs
             WHERE session_id = ?1
             ORDER BY started_at ASC",
        )?;
        let rows = stmt.query_map(params![session_id], row_to_task_run)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Return the most recent `limit` task runs for a session in chronological order.
    pub fn list_recent_task_runs_for_session(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<TaskRun>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, correlation_id, name, display_name, agent_type,
                    execution_mode, requested_model, actual_model, started_at, completed_at,
                    duration_ms, success, total_tokens, total_tool_calls,
                    description_size_chars, prompt_size_chars, summary_size_chars, confidence
             FROM task_runs
             WHERE session_id = ?1
             ORDER BY started_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![session_id, limit as i64], row_to_task_run)?;
        let mut tasks = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        tasks.reverse();
        Ok(tasks)
    }

    /// Return the most recent `limit` turns for a session in chronological order.
    pub fn list_recent_turns_for_session(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<Turn>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, turn_index, timestamp, duration_ms,
                    input_tokens, cache_read_tokens, cache_write_tokens,
                    cache_write_5m_tokens, cache_write_1h_tokens, output_tokens,
                    thinking_tokens, mcp_call_count, mcp_input_token_est, text_output_tokens,
                    model, service_tier, estimated_cost_usd, is_compaction_event
             FROM turns
             WHERE session_id = ?1
             ORDER BY turn_index DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![session_id, limit as i64], row_to_turn)?;
        let mut turns = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        turns.reverse();
        Ok(turns)
    }

    /// Count turns after the most recent compaction marker for a session.
    pub fn count_turns_since_last_compaction(&self, session_id: &str) -> Result<i64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*)
             FROM turns
             WHERE session_id = ?1
               AND turn_index > COALESCE(
                   (SELECT MAX(turn_index) FROM turns WHERE session_id = ?1 AND is_compaction_event = 1),
                   -1
               )",
            params![session_id],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    pub fn list_recent_task_runs(&self, limit: usize) -> Result<Vec<TaskRun>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, correlation_id, name, display_name, agent_type,
                    execution_mode, requested_model, actual_model, started_at, completed_at,
                    duration_ms, success, total_tokens, total_tool_calls,
                    description_size_chars, prompt_size_chars, summary_size_chars, confidence
             FROM task_runs
             ORDER BY started_at DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], row_to_task_run)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn get_project_stats(&self) -> Result<Vec<ProjectStats>> {
        let mut stmt = self.conn.prepare(
            "SELECT
                s.project_name,
                s.git_branch,
                COUNT(DISTINCT s.id) AS session_count,
                COALESCE(SUM(t.estimated_cost_usd), 0.0) AS total_cost_usd,
                CASE WHEN SUM(t.input_tokens + t.cache_read_tokens + t.cache_write_tokens) > 0
                     THEN CAST(SUM(t.cache_read_tokens) AS REAL)
                          / SUM(t.input_tokens + t.cache_read_tokens + t.cache_write_tokens) * 100.0
                     ELSE 0.0 END AS avg_cache_hit_rate,
                COUNT(t.id) AS total_turns,
                COALESCE(SUM(t.is_compaction_event), 0) AS compaction_count,
                MAX(s.last_turn_at) AS most_recent_at
             FROM sessions s
             LEFT JOIN turns t ON t.session_id = s.id
             GROUP BY s.project_name, s.git_branch
             ORDER BY total_cost_usd DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ProjectStats {
                project_name: row.get(0)?,
                git_branch: row.get(1)?,
                session_count: row.get(2)?,
                total_cost_usd: row.get(3)?,
                avg_cache_hit_rate: row.get(4)?,
                total_turns: row.get(5)?,
                compaction_count: row.get(6)?,
                most_recent_at: row.get(7)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn get_tool_stats(&self, session_id: Option<&str>) -> Result<Vec<ToolStat>> {
        let sql = if session_id.is_some() {
            "SELECT tool_name,
                    COUNT(*) AS call_count,
                    AVG(CAST(input_size_chars AS REAL)) AS avg_input_chars
             FROM tool_calls
             WHERE session_id = ?1
             GROUP BY tool_name
             ORDER BY call_count DESC"
        } else {
            "SELECT tool_name,
                    COUNT(*) AS call_count,
                    AVG(CAST(input_size_chars AS REAL)) AS avg_input_chars
             FROM tool_calls
             GROUP BY tool_name
             ORDER BY call_count DESC"
        };

        // Sonnet-class pricing used as a conservative estimate for tool input cost.
        // Actual cost depends on the model used during the session.
        let input_price_per_mtok = 3.0f64; // Claude Sonnet 4 input price
        let cost_model_label = "sonnet-class-estimate";
        let mtok = 1_000_000.0f64;

        let map_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<ToolStat> {
            let tool_name: String = row.get(0)?;
            let call_count: i64 = row.get(1)?;
            let avg_input_chars: f64 = row.get::<_, f64>(2).unwrap_or(0.0);
            let est_tokens = (avg_input_chars / 4.0 * call_count as f64) as i64;
            let est_cost_usd = est_tokens as f64 / mtok * input_price_per_mtok;
            Ok(ToolStat {
                tool_name,
                call_count,
                avg_input_chars,
                est_tokens,
                est_cost_usd,
                cost_model: cost_model_label.to_string(),
            })
        };

        if let Some(sid) = session_id {
            let mut stmt = self.conn.prepare(sql)?;
            let rows = stmt.query_map(params![sid], map_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(Into::into)
        } else {
            let mut stmt = self.conn.prepare(sql)?;
            let rows = stmt.query_map([], map_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(Into::into)
        }
    }

    pub fn get_session_anomalies(&self) -> Result<Vec<SessionAnomaly>> {
        let median_cost: f64 = {
            let mut costs: Vec<f64> = {
                let mut stmt = self.conn.prepare(
                    "SELECT COALESCE(SUM(t.estimated_cost_usd), 0.0)
                     FROM sessions s
                     LEFT JOIN turns t ON t.session_id = s.id
                     GROUP BY s.id",
                )?;
                let rows = stmt.query_map([], |r| r.get::<_, f64>(0))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()?
            };
            costs.sort_by(|a, b| a.partial_cmp(b).unwrap());
            if costs.is_empty() {
                0.0
            } else {
                let mid = costs.len() / 2;
                #[allow(clippy::manual_is_multiple_of)]
                // `is_multiple_of` requires Rust ≥ 1.87; MSRV is 1.86
                if costs.len() % 2 == 0 {
                    (costs[mid - 1] + costs[mid]) / 2.0
                } else {
                    costs[mid]
                }
            }
        };

        let mut stmt = self.conn.prepare(
            "SELECT
                s.id,
                COALESCE(SUM(t.estimated_cost_usd), 0.0) AS session_cost,
                CASE WHEN COUNT(t.id) > 0 THEN CAST(SUM(t.mcp_call_count) AS REAL) / COUNT(t.id) ELSE 0.0 END AS mcp_density,
                CASE WHEN SUM(t.input_tokens + t.cache_read_tokens + t.cache_write_tokens) > 0
                     THEN CAST(SUM(t.cache_read_tokens) AS REAL) / SUM(t.input_tokens + t.cache_read_tokens + t.cache_write_tokens) * 100.0
                     ELSE 0.0 END AS cache_pct,
                COUNT(t.id) AS turn_count,
                s.had_compaction
             FROM sessions s
             LEFT JOIN turns t ON t.session_id = s.id
             GROUP BY s.id",
        )?;

        let threshold = median_cost * 2.0;
        let rows = stmt.query_map([], |row| {
            let session_id: String = row.get(0)?;
            let session_cost: f64 = row.get(1)?;
            let mcp_density: f64 = row.get(2)?;
            let cache_pct: f64 = row.get(3)?;
            let turn_count: i64 = row.get(4)?;
            let had_compaction: i64 = row.get(5)?;
            Ok((
                session_id,
                session_cost,
                mcp_density,
                cache_pct,
                turn_count,
                had_compaction,
            ))
        })?;

        let mut result = Vec::new();
        for row in rows {
            let (sid, cost, mcp_density, cache_pct, turns, had_compact) = row?;
            let is_expensive = threshold > 0.0 && cost > threshold;
            let is_tool_heavy = mcp_density > 5.0;
            let is_cache_cold = cache_pct < 10.0 && turns > 5;
            let had_compaction = had_compact != 0;
            result.push(SessionAnomaly {
                session_id: sid,
                is_expensive,
                is_tool_heavy,
                is_cache_cold,
                had_compaction,
            });
        }
        Ok(result)
    }
}

// ── Agent tree ────────────────────────────────────────────────────────────────

impl Database {
    /// Return the IDs of all sessions that are parents of at least one subagent.
    pub fn get_agent_root_ids(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT parent_session_id FROM sessions WHERE parent_session_id IS NOT NULL",
        )?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect::<rusqlite::Result<Vec<String>>>()
            .map_err(Into::into)
    }

    /// Recursively build the full agent tree rooted at `root_id`.
    pub fn get_agent_tree(&self, root_id: &str) -> Result<AgentNode> {
        struct Row {
            id: String,
            parent_id: String,
            project_name: String,
            model: String,
            cost: f64,
            turns: i64,
            tokens: i64,
            is_subagent: bool,
            started_at: i64,
        }

        let mut stmt = self.conn.prepare(
            "WITH RECURSIVE tree(id) AS (
                 SELECT id FROM sessions WHERE id = ?1
                 UNION ALL
                 SELECT s.id FROM sessions s JOIN tree t ON s.parent_session_id = t.id
             )
             SELECT
                 s.id,
                 COALESCE(s.parent_session_id, ''),
                 s.project_name,
                 s.model,
                 COALESCE(SUM(t.estimated_cost_usd), 0.0),
                 COUNT(t.id),
                 COALESCE(SUM(t.input_tokens + t.cache_read_tokens + t.output_tokens), 0),
                 s.is_subagent,
                 s.started_at
             FROM sessions s
             LEFT JOIN turns t ON t.session_id = s.id
             WHERE s.id IN (SELECT id FROM tree)
             GROUP BY s.id",
        )?;
        let nodes: Vec<Row> = stmt
            .query_map([root_id], |row| {
                Ok(Row {
                    id: row.get(0)?,
                    parent_id: row.get(1)?,
                    project_name: row.get(2)?,
                    model: row.get(3)?,
                    cost: row.get(4)?,
                    turns: row.get(5)?,
                    tokens: row.get(6)?,
                    is_subagent: row.get::<_, i32>(7)? != 0,
                    started_at: row.get(8)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        // Build a flat map then assemble the tree
        use std::collections::HashMap;
        let mut node_map: HashMap<String, AgentNode> = nodes
            .iter()
            .map(|r| {
                (
                    r.id.clone(),
                    AgentNode {
                        session_id: r.id.clone(),
                        project_name: r.project_name.clone(),
                        model: r.model.clone(),
                        turn_count: r.turns,
                        total_cost_usd: r.cost,
                        total_tokens: r.tokens,
                        is_subagent: r.is_subagent,
                        started_at: r.started_at,
                        children: Vec::new(),
                    },
                )
            })
            .collect();

        // Two-pass tree assembly that handles arbitrary depth.
        // The SQL recursive CTE returns nodes in BFS order (root first, leaves last).
        // We sort edges by the child's BFS index in DESCENDING order so the deepest
        // nodes are linked first — when a parent is later removed from node_map and
        // placed into its own parent, it already carries its children with it.
        let mut edges: Vec<(String, String)> = nodes
            .iter()
            .filter(|r| !r.parent_id.is_empty())
            .map(|r| (r.parent_id.clone(), r.id.clone()))
            .collect();
        let depth_proxy: HashMap<&str, usize> = nodes
            .iter()
            .enumerate()
            .map(|(i, r)| (r.id.as_str(), i))
            .collect();
        edges.sort_by_key(|(_, child_id)| {
            std::cmp::Reverse(depth_proxy.get(child_id.as_str()).copied().unwrap_or(0))
        });
        for (parent_id, child_id) in &edges {
            if let Some(child) = node_map.remove(child_id) {
                if let Some(parent) = node_map.get_mut(parent_id) {
                    parent.children.push(child);
                }
            }
        }

        node_map
            .remove(root_id)
            .ok_or_else(|| anyhow::anyhow!("root session '{}' not found", root_id))
    }

    // ── Session tags (M0004) ─────────────────────────────────────────────────

    /// Set tags for a session, replacing any previous tags.
    pub fn set_session_tags(&self, session_id: &str, tags: &[&str]) -> Result<()> {
        self.conn.execute(
            "DELETE FROM session_tags WHERE session_id = ?1",
            params![session_id],
        )?;
        for tag in tags {
            let tag = tag.trim();
            if !tag.is_empty() {
                self.conn.execute(
                    "INSERT OR IGNORE INTO session_tags (session_id, tag) VALUES (?1, ?2)",
                    params![session_id, tag],
                )?;
            }
        }
        Ok(())
    }

    /// Get tags for a session.
    pub fn get_session_tags(&self, session_id: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT tag FROM session_tags WHERE session_id = ?1 ORDER BY tag")?;
        let tags: Vec<String> = stmt
            .query_map(params![session_id], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(tags)
    }

    /// Get all sessions with a given tag.
    pub fn get_sessions_by_tag(&self, tag: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT session_id FROM session_tags WHERE tag = ?1 ORDER BY session_id")?;
        let ids: Vec<String> = stmt
            .query_map(params![tag], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(ids)
    }

    /// Get cost aggregated by tag. Returns Vec<(tag, total_cost_usd, session_count)>.
    /// Cost grouped by model, descending. Only returns models with cost > 0.
    pub fn get_cost_by_model(&self) -> Result<Vec<(String, f64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT model, COALESCE(SUM(estimated_cost_usd), 0.0) AS total_cost
             FROM turns
             WHERE model IS NOT NULL AND model != ''
             GROUP BY model
             ORDER BY total_cost DESC, COUNT(*) DESC",
        )?;
        let rows: Vec<_> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_cost_by_tag(&self) -> Result<Vec<(String, f64, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT st.tag,
                    SUM(t.estimated_cost_usd) AS total_cost,
                    COUNT(DISTINCT st.session_id) AS session_count
             FROM session_tags st
             JOIN turns t ON t.session_id = st.session_id
             GROUP BY st.tag
             ORDER BY total_cost DESC",
        )?;
        let rows: Vec<_> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Returns (model, total_cache_read_tokens, total_cache_write_tokens) for
    /// all turns in the database. Used by digest to compute exact cache savings
    /// with model-specific pricing via `cache_savings_usd()`.
    pub fn get_cache_tokens_by_model(&self) -> Result<Vec<(String, i64, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT model,
                    COALESCE(SUM(cache_read_tokens), 0) AS total_read,
                    COALESCE(SUM(cache_write_tokens), 0) AS total_write
             FROM turns
             WHERE model IS NOT NULL AND model != ''
             GROUP BY model",
        )?;
        let rows: Vec<_> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
}

// ── Reprice support ───────────────────────────────────────────────────────────

impl Database {
    /// Return the minimal turn fields needed to recalculate costs.
    pub fn list_all_turns_for_reprice(&self) -> Result<Vec<Turn>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, turn_index, timestamp, duration_ms,
                    input_tokens, cache_read_tokens, cache_write_tokens,
                    cache_write_5m_tokens, cache_write_1h_tokens, output_tokens,
                    thinking_tokens, mcp_call_count, mcp_input_token_est, text_output_tokens,
                    model, service_tier, estimated_cost_usd, is_compaction_event
             FROM turns",
        )?;
        let rows = stmt.query_map([], row_to_turn)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Update the estimated_cost_usd for a single turn.
    pub fn update_turn_cost(&self, turn_id: &str, cost_usd: f64) -> Result<()> {
        self.conn.execute(
            "UPDATE turns SET estimated_cost_usd = ?1 WHERE id = ?2",
            params![cost_usd, turn_id],
        )?;
        Ok(())
    }
}

// ── Row mappers ───────────────────────────────────────────────────────────────

fn row_to_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<Session> {
    Ok(Session {
        id: row.get(0)?,
        project: row.get(1)?,
        project_name: row.get(2)?,
        slug: row.get(3)?,
        provider: row.get(4)?,
        provider_version: row.get(5)?,
        model: row.get(6)?,
        git_branch: row.get(7)?,
        started_at: row.get(8)?,
        last_turn_at: row.get(9)?,
        total_turns: row.get(10)?,
        is_subagent: row.get::<_, i32>(11)? != 0,
        parent_session_id: row.get(12)?,
        context_window_tokens: row.get(13)?,
    })
}

fn row_to_interaction_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<InteractionEvent> {
    Ok(InteractionEvent {
        id: row.get(0)?,
        session_id: row.get(1)?,
        turn_id: row.get(2)?,
        task_run_id: row.get(3)?,
        correlation_id: row.get(4)?,
        parent_id: row.get(5)?,
        provider: row.get(6)?,
        timestamp: row.get(7)?,
        kind: row.get(8)?,
        phase: row.get(9)?,
        name: row.get(10)?,
        display_name: row.get(11)?,
        mcp_server: row.get(12)?,
        mcp_tool: row.get(13)?,
        hook_type: row.get(14)?,
        agent_type: row.get(15)?,
        execution_mode: row.get(16)?,
        model: row.get(17)?,
        status: row.get(18)?,
        success: row.get::<_, Option<i32>>(19)?.map(|v| v != 0),
        input_size_chars: row.get(20)?,
        output_size_chars: row.get(21)?,
        prompt_size_chars: row.get(22)?,
        summary_size_chars: row.get(23)?,
        total_tokens: row.get(24)?,
        total_tool_calls: row.get(25)?,
        duration_ms: row.get(26)?,
        estimated_input_tokens: row.get(27)?,
        estimated_output_tokens: row.get(28)?,
        estimated_cost_usd: row.get(29)?,
        confidence: row.get(30)?,
    })
}

fn row_to_task_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskRun> {
    Ok(TaskRun {
        id: row.get(0)?,
        session_id: row.get(1)?,
        correlation_id: row.get(2)?,
        name: row.get(3)?,
        display_name: row.get(4)?,
        agent_type: row.get(5)?,
        execution_mode: row.get(6)?,
        requested_model: row.get(7)?,
        actual_model: row.get(8)?,
        started_at: row.get(9)?,
        completed_at: row.get(10)?,
        duration_ms: row.get(11)?,
        success: row.get::<_, Option<i32>>(12)?.map(|v| v != 0),
        total_tokens: row.get(13)?,
        total_tool_calls: row.get(14)?,
        description_size_chars: row.get(15)?,
        prompt_size_chars: row.get(16)?,
        summary_size_chars: row.get(17)?,
        confidence: row.get(18)?,
    })
}

fn row_to_turn(row: &rusqlite::Row<'_>) -> rusqlite::Result<Turn> {
    Ok(Turn {
        id: row.get(0)?,
        session_id: row.get(1)?,
        turn_index: row.get(2)?,
        timestamp: row.get(3)?,
        duration_ms: row.get(4)?,
        input_tokens: row.get(5)?,
        cache_read_tokens: row.get(6)?,
        cache_write_tokens: row.get(7)?,
        cache_write_5m_tokens: row.get(8)?,
        cache_write_1h_tokens: row.get(9)?,
        output_tokens: row.get(10)?,
        thinking_tokens: row.get(11)?,
        mcp_call_count: row.get(12)?,
        mcp_input_token_est: row.get(13)?,
        text_output_tokens: row.get(14)?,
        model: row.get(15)?,
        service_tier: row.get(16)?,
        estimated_cost_usd: row.get(17)?,
        is_compaction_event: row.get::<_, i32>(18)? != 0,
    })
}

fn row_to_daily(row: &rusqlite::Row<'_>) -> rusqlite::Result<DailyRollup> {
    Ok(DailyRollup {
        date: row.get(0)?,
        total_input_tokens: row.get(1)?,
        total_cache_read_tokens: row.get(2)?,
        total_cache_write_tokens: row.get(3)?,
        total_output_tokens: row.get(4)?,
        total_thinking_tokens: row.get(5)?,
        total_mcp_calls: row.get(6)?,
        session_count: row.get(7)?,
        turn_count: row.get(8)?,
        estimated_cost_usd: row.get(9)?,
        // M0005: health_score_avg — default 0.0 for rows written before migration
        health_score_avg: row.get(10).unwrap_or(0.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_session(id: &str) -> Session {
        Session {
            id: id.to_string(),
            project: "/home/user/project".to_string(),
            project_name: "project".to_string(),
            slug: "test-session".to_string(),
            provider: "claude-code".to_string(),
            provider_version: "2.1.15".to_string(),
            model: "claude-opus-4-5-20251101".to_string(),
            git_branch: "main".to_string(),
            started_at: 1_700_000_000_000,
            last_turn_at: 1_700_000_001_000,
            total_turns: 1,
            is_subagent: false,
            parent_session_id: None,
            context_window_tokens: None,
        }
    }

    fn make_turn(id: &str, session_id: &str, index: i64) -> Turn {
        Turn {
            id: id.to_string(),
            session_id: session_id.to_string(),
            turn_index: index,
            timestamp: 1_700_000_000_000 + index * 1000,
            duration_ms: Some(1500),
            input_tokens: 100,
            cache_read_tokens: 500,
            cache_write_tokens: 200,
            cache_write_5m_tokens: 200,
            cache_write_1h_tokens: 0,
            output_tokens: 50,
            thinking_tokens: 30,
            mcp_call_count: 2,
            mcp_input_token_est: 10,
            text_output_tokens: 20,
            model: "claude-opus-4-5-20251101".to_string(),
            service_tier: "standard".to_string(),
            estimated_cost_usd: 0.005,
            is_compaction_event: false,
        }
    }

    fn make_task_run(id: &str, session_id: &str) -> TaskRun {
        TaskRun {
            id: id.to_string(),
            session_id: session_id.to_string(),
            correlation_id: Some(format!("corr-{id}")),
            name: "task".to_string(),
            display_name: Some("Background task".to_string()),
            agent_type: "task".to_string(),
            execution_mode: "background".to_string(),
            requested_model: Some("claude-sonnet-4.5".to_string()),
            actual_model: Some("claude-sonnet-4.5".to_string()),
            started_at: 1_700_000_000_100,
            completed_at: Some(1_700_000_001_100),
            duration_ms: Some(1000),
            success: Some(true),
            total_tokens: Some(4096),
            total_tool_calls: Some(6),
            description_size_chars: 32,
            prompt_size_chars: 256,
            summary_size_chars: 64,
            confidence: "exact".to_string(),
        }
    }

    fn make_interaction_event(
        id: &str,
        session_id: &str,
        turn_id: &str,
        task_run_id: &str,
    ) -> InteractionEvent {
        InteractionEvent {
            id: id.to_string(),
            session_id: session_id.to_string(),
            turn_id: Some(turn_id.to_string()),
            task_run_id: Some(task_run_id.to_string()),
            correlation_id: Some("tool-1".to_string()),
            parent_id: None,
            provider: "copilot-cli".to_string(),
            timestamp: 1_700_000_000_500,
            kind: "mcp".to_string(),
            phase: "complete".to_string(),
            name: "gitnexus-query".to_string(),
            display_name: Some("GitNexus Query".to_string()),
            mcp_server: Some("gitnexus".to_string()),
            mcp_tool: Some("query".to_string()),
            hook_type: None,
            agent_type: Some("task".to_string()),
            execution_mode: Some("background".to_string()),
            model: Some("claude-sonnet-4.5".to_string()),
            status: Some("completed".to_string()),
            success: Some(true),
            input_size_chars: 120,
            output_size_chars: 80,
            prompt_size_chars: 0,
            summary_size_chars: 0,
            total_tokens: Some(256),
            total_tool_calls: Some(1),
            duration_ms: Some(250),
            estimated_input_tokens: 30,
            estimated_output_tokens: 20,
            estimated_cost_usd: 0.001,
            confidence: "estimated".to_string(),
        }
    }

    #[test]
    fn test_open_in_memory() {
        let db = Database::open_in_memory().expect("in-memory DB should open");
        // Verify schema exists by querying it
        let count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_upsert_and_get_session() {
        let db = Database::open_in_memory().unwrap();
        let session = make_session("sess-1");
        db.upsert_session(&session).unwrap();

        let fetched = db
            .get_session("sess-1")
            .unwrap()
            .expect("session should exist");
        assert_eq!(fetched.id, "sess-1");
        assert_eq!(fetched.project_name, "project");
        assert_eq!(fetched.model, "claude-opus-4-5-20251101");
        assert!(!fetched.is_subagent);
    }

    #[test]
    fn test_upsert_session_is_idempotent() {
        let db = Database::open_in_memory().unwrap();
        let session = make_session("sess-2");
        db.upsert_session(&session).unwrap();
        // Upserting again should not fail or duplicate
        db.upsert_session(&session).unwrap();
        let sessions = db.list_sessions(100).unwrap();
        assert_eq!(sessions.len(), 1);
    }

    #[test]
    fn test_upsert_and_get_turn() {
        let db = Database::open_in_memory().unwrap();
        let session = make_session("sess-3");
        db.upsert_session(&session).unwrap();

        let turn = make_turn("turn-1", "sess-3", 0);
        db.upsert_turn(&turn).unwrap();

        let turns = db.list_turns_for_session("sess-3").unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].input_tokens, 100);
        assert_eq!(turns[0].cache_read_tokens, 500);
        assert_eq!(turns[0].mcp_call_count, 2);
    }

    #[test]
    fn test_turn_insert_is_idempotent() {
        let db = Database::open_in_memory().unwrap();
        let session = make_session("sess-4");
        db.upsert_session(&session).unwrap();
        let turn = make_turn("turn-2", "sess-4", 0);
        db.upsert_turn(&turn).unwrap();
        db.upsert_turn(&turn).unwrap(); // second insert ignored
        assert_eq!(db.list_turns_for_session("sess-4").unwrap().len(), 1);
    }

    #[test]
    fn test_get_session_stats_aggregates_correctly() {
        let db = Database::open_in_memory().unwrap();
        let session = make_session("sess-5");
        db.upsert_session(&session).unwrap();

        for i in 0..3 {
            let turn = make_turn(&format!("t-{}", i), "sess-5", i);
            db.upsert_turn(&turn).unwrap();
        }

        let stats = db.get_session_stats("sess-5").unwrap();
        assert_eq!(stats.total_turns, 3);
        assert_eq!(stats.total_input_tokens, 300); // 3 × 100
        assert_eq!(stats.total_cache_read_tokens, 1500); // 3 × 500
        assert_eq!(stats.total_mcp_calls, 6); // 3 × 2
        assert!((stats.estimated_cost_usd - 0.015).abs() < 1e-9);
    }

    #[test]
    fn test_cache_hit_rate_calculation() {
        let db = Database::open_in_memory().unwrap();
        let session = make_session("sess-6");
        db.upsert_session(&session).unwrap();
        db.upsert_turn(&make_turn("t-a", "sess-6", 0)).unwrap();

        let stats = db.get_session_stats("sess-6").unwrap();
        // cache_hit_rate = cache_read / (input + cache_read + cache_write)
        // = 500 / (100 + 500 + 200) = 500/800 = 62.5%
        assert!((stats.cache_hit_rate - 500.0 / 800.0).abs() < 1e-9);
    }

    #[test]
    fn test_file_offset_tracking() {
        let db = Database::open_in_memory().unwrap();
        assert_eq!(db.get_file_offset("/some/file.jsonl").unwrap(), 0);

        db.set_file_offset("/some/file.jsonl", 1024, 9999).unwrap();
        assert_eq!(db.get_file_offset("/some/file.jsonl").unwrap(), 1024);

        // Update offset
        db.set_file_offset("/some/file.jsonl", 2048, 10000).unwrap();
        assert_eq!(db.get_file_offset("/some/file.jsonl").unwrap(), 2048);
    }

    #[test]
    fn test_global_stats_empty_db() {
        let db = Database::open_in_memory().unwrap();
        let stats = db.get_global_stats().unwrap();
        assert_eq!(stats.total_sessions, 0);
        assert_eq!(stats.total_turns, 0);
        assert_eq!(stats.cache_hit_rate, 0.0);
        assert_eq!(stats.estimated_cost_usd, 0.0);
    }

    #[test]
    fn test_daily_rollup_refresh() {
        let db = Database::open_in_memory().unwrap();
        let session = make_session("sess-7");
        db.upsert_session(&session).unwrap();

        // Insert turns with a known timestamp (2023-11-14 UTC)
        let mut turn = make_turn("tr-0", "sess-7", 0);
        turn.timestamp = 1_699_920_000_000; // 2023-11-14 00:00:00 UTC
        db.upsert_turn(&turn).unwrap();

        db.refresh_daily_rollup().unwrap();
        let rollups = db.get_daily_rollups(30).unwrap();
        assert_eq!(rollups.len(), 1);
        assert_eq!(rollups[0].date, "2023-11-14");
        assert_eq!(rollups[0].turn_count, 1);
        assert_eq!(rollups[0].total_input_tokens, 100);
    }

    #[test]
    fn test_latest_session_id_none_on_empty() {
        let db = Database::open_in_memory().unwrap();
        assert!(db.get_latest_session_id().unwrap().is_none());
    }

    #[test]
    fn test_latest_session_id_returns_most_recent() {
        let db = Database::open_in_memory().unwrap();
        let mut s1 = make_session("old-session");
        s1.last_turn_at = 1_000;
        let mut s2 = make_session("new-session");
        s2.last_turn_at = 2_000;
        db.upsert_session(&s1).unwrap();
        db.upsert_session(&s2).unwrap();

        let latest = db.get_latest_session_id().unwrap().unwrap();
        assert_eq!(latest, "new-session");
    }

    #[test]
    fn test_upsert_session_updates_subagent_fields_on_conflict() {
        let db = Database::open_in_memory().unwrap();
        // Insert as a root session first
        let session = make_session("child-sess");
        db.upsert_session(&session).unwrap();

        // Now re-upsert with is_subagent = true and a parent
        let mut updated = make_session("child-sess");
        updated.is_subagent = true;
        updated.parent_session_id = Some("parent-sess".to_string());
        db.upsert_session(&updated).unwrap();

        let fetched = db.get_session("child-sess").unwrap().unwrap();
        assert!(
            fetched.is_subagent,
            "is_subagent should be updated on conflict"
        );
        assert_eq!(
            fetched.parent_session_id.as_deref(),
            Some("parent-sess"),
            "parent_session_id should be updated on conflict"
        );
    }

    #[test]
    fn test_session_anomaly_detects_expensive_session() {
        let db = Database::open_in_memory().unwrap();
        // Insert cheap sessions to establish a low median
        for i in 0..4 {
            let sess = make_session(&format!("cheap-{}", i));
            db.upsert_session(&sess).unwrap();
            let mut turn = make_turn(&format!("tc-{}", i), &format!("cheap-{}", i), 0);
            turn.estimated_cost_usd = 0.001;
            db.upsert_turn(&turn).unwrap();
        }
        // Insert one expensive session (cost >> 2× median)
        let expensive = make_session("expensive-sess");
        db.upsert_session(&expensive).unwrap();
        let mut exp_turn = make_turn("te-0", "expensive-sess", 0);
        exp_turn.estimated_cost_usd = 10.0;
        db.upsert_turn(&exp_turn).unwrap();

        let anomalies = db.get_session_anomalies().unwrap();
        let expensive_anomaly = anomalies.iter().find(|a| a.session_id == "expensive-sess");
        assert!(
            expensive_anomaly.is_some(),
            "expensive session should appear in anomalies"
        );
        assert!(
            expensive_anomaly.unwrap().is_expensive,
            "expensive session should be flagged is_expensive"
        );
    }

    #[test]
    fn test_session_anomaly_detects_tool_heavy_session() {
        let db = Database::open_in_memory().unwrap();
        let sess = make_session("tool-heavy");
        db.upsert_session(&sess).unwrap();
        // mcp_call_count > 5.0 average = tool heavy
        for i in 0..3 {
            let mut turn = make_turn(&format!("th-{}", i), "tool-heavy", i);
            turn.mcp_call_count = 10;
            db.upsert_turn(&turn).unwrap();
        }

        let anomalies = db.get_session_anomalies().unwrap();
        let anomaly = anomalies
            .iter()
            .find(|a| a.session_id == "tool-heavy")
            .unwrap();
        assert!(anomaly.is_tool_heavy);
    }

    #[test]
    fn test_list_all_turns_for_reprice() {
        let db = Database::open_in_memory().unwrap();
        let sess = make_session("reprice-sess");
        db.upsert_session(&sess).unwrap();
        for i in 0..5 {
            db.upsert_turn(&make_turn(&format!("rp-{}", i), "reprice-sess", i))
                .unwrap();
        }

        let turns = db.list_all_turns_for_reprice().unwrap();
        assert_eq!(turns.len(), 5);
    }

    #[test]
    fn test_update_turn_cost() {
        let db = Database::open_in_memory().unwrap();
        let sess = make_session("cost-sess");
        db.upsert_session(&sess).unwrap();
        db.upsert_turn(&make_turn("upd-0", "cost-sess", 0)).unwrap();

        db.update_turn_cost("upd-0", 99.99).unwrap();

        let turns = db.list_turns_for_session("cost-sess").unwrap();
        assert!((turns[0].estimated_cost_usd - 99.99).abs() < 1e-9);
    }

    #[test]
    fn test_get_project_stats_aggregates_by_project() {
        let db = Database::open_in_memory().unwrap();
        // Two sessions in the same project
        for i in 0..2 {
            let mut sess = make_session(&format!("proj-sess-{}", i));
            sess.project_name = "my-project".to_string();
            db.upsert_session(&sess).unwrap();
            db.upsert_turn(&make_turn(
                &format!("ps-{}", i),
                &format!("proj-sess-{}", i),
                0,
            ))
            .unwrap();
        }

        let stats = db.get_project_stats().unwrap();
        assert!(!stats.is_empty());
        let proj = stats.iter().find(|s| s.project_name == "my-project");
        assert!(proj.is_some());
        assert_eq!(proj.unwrap().total_turns, 2);
    }

    #[test]
    fn test_tool_call_upsert_and_list() {
        let db = Database::open_in_memory().unwrap();
        let sess = make_session("tc-sess");
        db.upsert_session(&sess).unwrap();
        db.upsert_turn(&make_turn("tc-turn-0", "tc-sess", 0))
            .unwrap();

        let tc = ToolCall {
            id: "tcall-1".to_string(),
            session_id: "tc-sess".to_string(),
            turn_id: "tc-turn-0".to_string(),
            tool_name: "bash".to_string(),
            input_size_chars: 42,
            input_hash: 0,
            timestamp: 1_700_000_000_000,
        };
        db.upsert_tool_call(&tc).unwrap();
        db.upsert_tool_call(&tc).unwrap(); // idempotent

        let calls = db.list_tool_calls_for_session("tc-sess").unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "bash");
        assert_eq!(calls[0].input_size_chars, 42);
    }

    #[test]
    fn test_task_run_and_interaction_event_upsert_and_list() {
        let db = Database::open_in_memory().unwrap();
        let sess = make_session("prov-sess");
        db.upsert_session(&sess).unwrap();
        db.upsert_turn(&make_turn("prov-turn-0", "prov-sess", 0))
            .unwrap();

        let task = make_task_run("task-1", "prov-sess");
        let event = make_interaction_event("evt-1", "prov-sess", "prov-turn-0", "task-1");

        db.upsert_task_run(&task).unwrap();
        db.upsert_task_run(&task).unwrap();
        db.upsert_interaction_event(&event).unwrap();
        db.upsert_interaction_event(&event).unwrap();

        let tasks = db.list_task_runs_for_session("prov-sess").unwrap();
        let events = db
            .list_interaction_events_for_session("prov-sess", 10)
            .unwrap();

        assert_eq!(tasks.len(), 1);
        assert_eq!(
            tasks[0].requested_model.as_deref(),
            Some("claude-sonnet-4.5")
        );
        assert_eq!(tasks[0].total_tool_calls, Some(6));

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].mcp_server.as_deref(), Some("gitnexus"));
        assert_eq!(events[0].task_run_id.as_deref(), Some("task-1"));
        assert_eq!(events[0].total_tokens, Some(256));
    }

    #[test]
    fn test_agent_tree_builds_parent_child_hierarchy() {
        let db = Database::open_in_memory().unwrap();

        let parent = make_session("parent-agent");
        db.upsert_session(&parent).unwrap();
        db.upsert_turn(&make_turn("pa-t0", "parent-agent", 0))
            .unwrap();

        let mut child = make_session("child-agent");
        child.is_subagent = true;
        child.parent_session_id = Some("parent-agent".to_string());
        db.upsert_session(&child).unwrap();
        db.upsert_turn(&make_turn("ch-t0", "child-agent", 0))
            .unwrap();

        let roots = db.get_agent_root_ids().unwrap();
        assert!(roots.contains(&"parent-agent".to_string()));

        let tree = db.get_agent_tree("parent-agent").unwrap();
        assert_eq!(tree.session_id, "parent-agent");
        assert_eq!(tree.children.len(), 1);
        assert_eq!(tree.children[0].session_id, "child-agent");
        assert!(tree.children[0].is_subagent);
    }

    #[test]
    fn test_session_tags_set_and_get() {
        let db = Database::open_in_memory().unwrap();
        let s = make_session("s-tags-1");
        db.upsert_session(&s).unwrap();

        db.set_session_tags("s-tags-1", &["feat-auth", "sprint-12"])
            .unwrap();
        let tags = db.get_session_tags("s-tags-1").unwrap();
        assert_eq!(tags, vec!["feat-auth", "sprint-12"]);
    }

    #[test]
    fn test_session_tags_replace() {
        let db = Database::open_in_memory().unwrap();
        let s = make_session("s-tags-2");
        db.upsert_session(&s).unwrap();

        db.set_session_tags("s-tags-2", &["old-tag"]).unwrap();
        db.set_session_tags("s-tags-2", &["new-tag"]).unwrap();
        let tags = db.get_session_tags("s-tags-2").unwrap();
        assert_eq!(tags, vec!["new-tag"]);
    }

    #[test]
    fn test_session_tags_clear() {
        let db = Database::open_in_memory().unwrap();
        let s = make_session("s-tags-3");
        db.upsert_session(&s).unwrap();

        db.set_session_tags("s-tags-3", &["a", "b"]).unwrap();
        db.set_session_tags("s-tags-3", &[]).unwrap();
        let tags = db.get_session_tags("s-tags-3").unwrap();
        assert!(tags.is_empty());
    }

    #[test]
    fn test_get_cost_by_tag() {
        let db = Database::open_in_memory().unwrap();
        let s1 = make_session("s-cost-tag-1");
        let s2 = make_session("s-cost-tag-2");
        db.upsert_session(&s1).unwrap();
        db.upsert_session(&s2).unwrap();

        let mut t1 = make_turn("t-ct-1", "s-cost-tag-1", 0);
        t1.estimated_cost_usd = 0.50;
        db.upsert_turn(&t1).unwrap();
        let mut t2 = make_turn("t-ct-2", "s-cost-tag-2", 0);
        t2.estimated_cost_usd = 0.25;
        db.upsert_turn(&t2).unwrap();

        db.set_session_tags("s-cost-tag-1", &["feat"]).unwrap();
        db.set_session_tags("s-cost-tag-2", &["feat"]).unwrap();

        let rows = db.get_cost_by_tag().unwrap();
        let feat_row = rows.iter().find(|(tag, _, _)| tag == "feat");
        assert!(feat_row.is_some());
        let (_, cost, count) = feat_row.unwrap();
        assert!((cost - 0.75).abs() < 0.001);
        assert_eq!(*count, 2);
    }
}
