use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tokio::time;
use tracing::{error, info, warn};

use scopeon_core::{
    branch_to_tag, infer_tag_from_tool_calls, Database, COMPACTION_MIN_PREV_TOKENS,
};

use crate::parser::parse_file_incremental;
use crate::providers::Provider;

/// Process a single JSONL file incrementally (from last known offset).
pub fn process_file(file_path: &Path, db: &Database) -> Result<()> {
    let path_str = file_path.to_string_lossy().to_string();
    let (stored_offset, stored_mtime) = db.get_file_offset_and_mtime(&path_str)?;

    let metadata = std::fs::metadata(file_path)?;
    let file_size = metadata.len();

    // Detect file truncation (e.g. log rotation): if stored offset exceeds current
    // size AND the file was modified more recently than our last parse, re-parse
    // from the beginning.
    let current_mtime = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0);

    let from_offset = if stored_offset > file_size && current_mtime > stored_mtime {
        info!(
            "File truncated/replaced, re-parsing from start: {:?}",
            file_path
        );
        0
    } else {
        stored_offset
    };

    if from_offset >= file_size && from_offset > 0 {
        return Ok(()); // Nothing new
    }

    // For incremental parses, determine the turn offset so new turns get
    // correct indices and unique IDs (avoids session_id-0 collision).
    let start_turn_index = if from_offset > 0 {
        match peek_session_id(file_path) {
            Some(sid) => db.count_turns_for_session(&sid).unwrap_or(0),
            None => {
                // Log a warning but continue — fall back to 0 which may cause
                // duplicate turn IDs if session_id appears beyond the first lines.
                warn!(
                    "Could not find session ID in {:?}, turn indexing may be incorrect",
                    file_path
                );
                0
            },
        }
    } else {
        0
    };

    let result = parse_file_incremental(file_path, from_offset, start_turn_index)?;

    if result.skipped_lines > 0 {
        warn!(
            "{} unparseable line(s) skipped in {:?} — data may be incomplete",
            result.skipped_lines, file_path
        );
    }

    // Atomically write session, turns, tool calls, interaction events, and offset
    // in a single transaction — crash-safe.
    db.commit_parse_result(
        &path_str,
        result.session.as_ref(),
        &result.turns,
        &result.tool_calls,
        &result.interaction_events,
        result.new_offset,
        current_mtime,
    )?;

    // Auto-tag session from tool-call patterns (falls back to branch prefix).
    // Only applies a tag if no manual tag has been set and the session has tool calls.
    if let Some(ref session) = result.session {
        if db
            .get_session_tags(&session.id)
            .map(|t| t.is_empty())
            .unwrap_or(true)
        {
            let tag = infer_tag_from_tool_calls(&result.tool_calls)
                .or_else(|| branch_to_tag(&session.git_branch));
            if let Some(tag) = tag {
                if let Err(e) = db.set_session_tags(&session.id, &[tag]) {
                    tracing::debug!(
                        "Auto-tag '{}' failed for session {}: {}",
                        tag,
                        session.id,
                        e
                    );
                }
            }
        }
    }

    // §7.3: After turns are inserted, update session.total_turns from the authoritative
    // COUNT(*) query rather than trusting the parser's local counter.
    if let Some(ref session) = result.session {
        if let Ok(count) = db.count_turns_for_session(&session.id) {
            if count > 0 && count != session.total_turns {
                let mut updated = session.clone();
                updated.total_turns = count;
                let _ = db.upsert_session(&updated);
            }
        }
    }

    // Compaction detection: check ALL turns in the batch (not just the last one).
    // This avoids missing compaction events when multiple turns arrive in one batch
    // (common during backfill or after a period of inactivity).
    for (i, new_turn) in result.turns.iter().enumerate() {
        // Compare against the previous turn in the batch, or fall back to the DB
        // for the first turn in the batch.
        let prev_input: Option<i64> = if i > 0 {
            Some(result.turns[i - 1].input_tokens)
        } else {
            db.get_turn_input_before(&new_turn.session_id, new_turn.turn_index)
                .ok()
                .flatten()
        };
        if let Some(prev) = prev_input {
            // Compaction: >50% input-token drop from a turn that had >50k tokens.
            let threshold = (prev as f64 * 0.5) as i64;
            if prev > COMPACTION_MIN_PREV_TOKENS && new_turn.input_tokens < threshold {
                let _ = db.mark_session_had_compaction(&new_turn.session_id);
                tracing::debug!(
                    "Compaction detected at turn {} (prev={}, now={})",
                    new_turn.turn_index,
                    prev,
                    new_turn.input_tokens
                );
            }
        }
    }

    // Incrementally refresh only the calendar dates touched by this batch.
    // This is O(dates changed) instead of O(all turns), keeping the hot path fast.
    if !result.turns.is_empty() {
        let timestamps: Vec<i64> = result.turns.iter().map(|t| t.timestamp).collect();
        db.refresh_daily_rollup_for_timestamps(&timestamps)?;
    }

    Ok(())
}

/// Scan all existing JSONL files in a directory tree and insert any unprocessed data.
/// Intended for initial backfill at startup.
pub fn backfill_providers(providers: &[Box<dyn Provider>], db: &Database) -> Result<()> {
    info!("Starting provider-based backfill");
    let mut total = 0;
    for provider in providers {
        if !provider.is_available() {
            info!("Provider '{}' not available, skipping", provider.name());
            continue;
        }
        match provider.scan(db) {
            Ok(n) => {
                info!("Provider '{}' scanned {} items", provider.name(), n);
                total += n;
            },
            Err(e) => warn!("Provider '{}' scan error: {}", provider.name(), e),
        }
    }
    info!("Provider backfill complete: {} items processed", total);
    Ok(())
}

/// Background-friendly backfill: acquires the DB mutex per provider scan (or per
/// file for providers that override `scan_incremental`) so the TUI can refresh
/// in the gaps between lock acquisitions.
///
/// Use this instead of [`backfill_providers`] when the TUI is already running.
pub fn backfill_providers_arc(
    providers: &[Box<dyn Provider>],
    db: Arc<Mutex<Database>>,
) -> Result<()> {
    info!("Starting background provider backfill");
    let mut total = 0;
    for provider in providers {
        if !provider.is_available() {
            info!("Provider '{}' not available, skipping", provider.name());
            continue;
        }
        match provider.scan_incremental(db.clone()) {
            Ok(n) => {
                info!("Provider '{}' scanned {} items", provider.name(), n);
                total += n;
            },
            Err(e) => warn!("Provider '{}' scan error: {}", provider.name(), e),
        }
    }
    info!("Background backfill complete: {} items processed", total);
    Ok(())
}

/// Watch paths from all providers and dispatch on file changes.
pub async fn start_watching_providers(
    providers: Vec<Box<dyn Provider>>,
    db: Arc<Mutex<Database>>,
) -> Result<()> {
    let (tx, mut rx) = mpsc::channel::<PathBuf>(128);

    let mut watcher = RecommendedWatcher::new(
        move |res: notify::Result<Event>| {
            if let Ok(event) = res {
                if matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                    for path in event.paths {
                        let _ = tx.blocking_send(path);
                    }
                }
            }
        },
        notify::Config::default().with_poll_interval(Duration::from_secs(2)),
    )?;

    let all_paths: Vec<PathBuf> = providers.iter().flat_map(|p| p.watch_paths()).collect();
    for path in &all_paths {
        if path.exists() {
            watcher.watch(path, RecursiveMode::Recursive)?;
            info!("Watching {:?} for updates", path);
        }
    }

    let providers = Arc::new(providers);

    // Build per-provider watch path lists once at startup; reused inside the event
    // loop so we never call watch_paths() per event.
    let provider_watch_paths: Arc<Vec<Vec<PathBuf>>> =
        Arc::new(providers.iter().map(|p| p.watch_paths()).collect());

    let debounce = Duration::from_millis(250);
    let mut pending: HashSet<PathBuf> = HashSet::new();

    loop {
        if pending.is_empty() {
            match rx.recv().await {
                Some(path) => {
                    pending.insert(path);
                },
                None => break,
            }
        }
        // Drain any additional events that arrive within the debounce window.
        let deadline = time::Instant::now() + debounce;
        loop {
            match time::timeout_at(deadline, rx.recv()).await {
                Ok(Some(path)) => {
                    pending.insert(path);
                },
                Ok(None) => break,
                Err(_) => break,
            }
        }
        let changed: Vec<PathBuf> = pending.drain().collect();
        let db = db.clone();
        let providers = providers.clone();
        let provider_watch_paths = provider_watch_paths.clone();
        // Run one scan batch at a time so repeated file events cannot pile up
        // overlapping full-provider rescans against the same SQLite mutex.
        let worker = tokio::task::spawn_blocking(move || {
            let db = match db.lock() {
                Ok(guard) => guard,
                Err(_) => {
                    error!("Database mutex poisoned in watcher — skipping scan batch; restart Scopeon to recover.");
                    return;
                },
            };
            for (provider, watch_paths) in providers.iter().zip(provider_watch_paths.iter()) {
                if !provider.is_available() {
                    continue;
                }
                // Only scan this provider if one of the changed paths falls under
                // one of its watch directories.
                let relevant = changed
                    .iter()
                    .any(|changed_path| watch_paths.iter().any(|wp| changed_path.starts_with(wp)));
                if relevant {
                    if let Err(e) = provider.scan(&db) {
                        error!("Provider '{}' scan error: {}", provider.name(), e);
                    }
                }
            }
        });
        if let Err(e) = worker.await {
            error!("Watcher scan batch join error: {}", e);
        }
    }

    Ok(())
}

/// Read the first few lines of a JSONL file to extract the session ID.
fn peek_session_id(path: &Path) -> Option<String> {
    use std::io::BufRead;
    let file = std::fs::File::open(path).ok()?;
    let reader = std::io::BufReader::new(file);
    for line in reader.lines().take(20).flatten() {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&line) {
            if let Some(sid) = val.get("sessionId").and_then(|v| v.as_str()) {
                return Some(sid.to_string());
            }
        }
    }
    None
}
