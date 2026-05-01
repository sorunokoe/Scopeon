//! Integration test: full backfill pipeline
//!
//! Writes synthetic JSONL files to a temp directory, processes each file with
//! `process_file`, and verifies that the SQLite database contains the expected data.

use std::fs;
use std::io::Write;
use std::path::Path;

use scopeon_collector::watcher::process_file;
use scopeon_core::Database;

fn write_jsonl(dir: &Path, filename: &str, lines: &[&str]) {
    let path = dir.join(filename);
    let mut file = fs::File::create(&path).unwrap();
    for line in lines {
        writeln!(file, "{}", line).unwrap();
    }
}

/// Walk `dir` recursively and call `process_file` on every `.jsonl` file.
fn backfill_dir(dir: &Path, db: &Database) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                backfill_dir(&path, db);
            } else if path.extension().map(|e| e == "jsonl").unwrap_or(false) {
                process_file(&path, db).expect("process_file failed");
            }
        }
    }
    db.refresh_daily_rollup()
        .expect("refresh_daily_rollup failed");
}

fn make_turn_line(
    session_id: &str,
    msg_id: &str,
    input: i64,
    cache_read: i64,
    output: i64,
) -> String {
    serde_json::json!({
        "type": "assistant",
        "sessionId": session_id,
        "cwd": "/home/user/myproject",
        "slug": "test-slug",
        "gitBranch": "main",
        "timestamp": "2024-03-01T12:00:00Z",
        "durationMs": 800,
        "message": {
            "id": msg_id,
            "model": "claude-opus-4-5-20251101",
            "type": "message",
            "role": "assistant",
            "content": [{"type": "text", "text": "Hello"}],
            "usage": {
                "input_tokens": input,
                "cache_read_input_tokens": cache_read,
                "cache_creation_input_tokens": 0,
                "cache_creation": {"ephemeral_5m_input_tokens": 0, "ephemeral_1h_input_tokens": 0},
                "output_tokens": output,
                "service_tier": "standard"
            }
        }
    })
    .to_string()
}

#[test]
fn test_backfill_single_session() {
    let dir = tempfile::tempdir().unwrap();
    let projects_dir = dir.path().join("projects").join("my-project");
    fs::create_dir_all(&projects_dir).unwrap();

    // Write a session with 3 turns
    let lines: Vec<String> = (0..3)
        .map(|i| make_turn_line("sess-integration", &format!("msg-{}", i), 100, 500, 50))
        .collect();
    let line_refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
    write_jsonl(&projects_dir, "sess-integration.jsonl", &line_refs);

    let db = Database::open_in_memory().unwrap();
    backfill_dir(dir.path().join("projects").as_path(), &db);

    // Session should be created
    let session = db.get_session("sess-integration").unwrap();
    assert!(session.is_some(), "session should exist after backfill");
    let session = session.unwrap();
    assert_eq!(session.project_name, "myproject");
    assert_eq!(session.slug, "test-slug");
    assert_eq!(session.total_turns, 3);

    // 3 turns with correct token values
    let stats = db.get_session_stats("sess-integration").unwrap();
    assert_eq!(stats.total_turns, 3);
    assert_eq!(stats.total_input_tokens, 300); // 3 × 100
    assert_eq!(stats.total_cache_read_tokens, 1500); // 3 × 500
    assert_eq!(stats.total_output_tokens, 150); // 3 × 50
}

#[test]
fn test_backfill_multiple_sessions() {
    let dir = tempfile::tempdir().unwrap();

    // Two sessions in two different project directories
    for (proj, sess) in [("proj-a", "sess-a"), ("proj-b", "sess-b")] {
        let proj_dir = dir.path().join("projects").join(proj);
        fs::create_dir_all(&proj_dir).unwrap();
        let line = make_turn_line(sess, &format!("{}-msg-0", sess), 10, 20, 5);
        write_jsonl(&proj_dir, &format!("{}.jsonl", sess), &[line.as_str()]);
    }

    let db = Database::open_in_memory().unwrap();
    backfill_dir(dir.path().join("projects").as_path(), &db);

    let global = db.get_global_stats().unwrap();
    assert_eq!(global.total_sessions, 2);
    assert_eq!(global.total_turns, 2);
}

#[test]
fn test_backfill_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let projects_dir = dir.path().join("projects").join("proj");
    fs::create_dir_all(&projects_dir).unwrap();

    let line = make_turn_line("sess-idem", "msg-0", 50, 100, 25);
    write_jsonl(&projects_dir, "sess-idem.jsonl", &[line.as_str()]);

    let db = Database::open_in_memory().unwrap();

    // Run backfill twice — should not double-count
    backfill_dir(dir.path().join("projects").as_path(), &db);
    backfill_dir(dir.path().join("projects").as_path(), &db);

    let stats = db.get_session_stats("sess-idem").unwrap();
    assert_eq!(
        stats.total_turns, 1,
        "idempotent backfill must not duplicate turns"
    );
    assert_eq!(stats.total_input_tokens, 50);
}

#[test]
fn test_backfill_empty_projects_dir() {
    let dir = tempfile::tempdir().unwrap();
    let projects_dir = dir.path().join("projects");
    fs::create_dir_all(&projects_dir).unwrap();

    let db = Database::open_in_memory().unwrap();
    backfill_dir(&projects_dir, &db);

    let global = db.get_global_stats().unwrap();
    assert_eq!(global.total_sessions, 0);
    assert_eq!(global.total_turns, 0);
}

#[test]
fn test_backfill_daily_rollup_computed() {
    let dir = tempfile::tempdir().unwrap();
    let proj_dir = dir.path().join("projects").join("proj");
    fs::create_dir_all(&proj_dir).unwrap();

    let line = make_turn_line("sess-rollup", "msg-r", 100, 0, 40);
    write_jsonl(&proj_dir, "sess-rollup.jsonl", &[line.as_str()]);

    let db = Database::open_in_memory().unwrap();
    backfill_dir(dir.path().join("projects").as_path(), &db);

    let rollups = db.get_daily_rollups(30).unwrap();
    assert_eq!(
        rollups.len(),
        1,
        "should have exactly one daily rollup entry"
    );
    assert_eq!(rollups[0].turn_count, 1);
    assert_eq!(rollups[0].total_input_tokens, 100);
}
