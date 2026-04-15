//! Automatic session tagging derived from git branch names and tool-call patterns.
//!
//! Extracts a structured task-type label from the branch prefix convention
//! used by most teams (e.g. `feat/auth-flow` → `"feature"`), and also from
//! observed tool-call patterns in a session (TRIZ D8).

use crate::ToolCall;

/// Derive a task-type tag from a git branch name using common prefix conventions.
///
/// Returns `None` for default branches (`main`, `master`, `develop`, `trunk`)
/// and for branch names that carry no recognisable task-type prefix.
///
/// # Examples
///
/// ```
/// use scopeon_core::branch_to_tag;
///
/// assert_eq!(branch_to_tag("feat/auth-flow"),      Some("feature"));
/// assert_eq!(branch_to_tag("feature/add-logging"), Some("feature"));
/// assert_eq!(branch_to_tag("fix/cache-bug"),       Some("bugfix"));
/// assert_eq!(branch_to_tag("refactor/db-layer"),   Some("refactor"));
/// assert_eq!(branch_to_tag("chore/update-deps"),   Some("chore"));
/// assert_eq!(branch_to_tag("docs/api-readme"),     Some("docs"));
/// assert_eq!(branch_to_tag("test/unit-suite"),     Some("test"));
/// assert_eq!(branch_to_tag("perf/faster-queries"), Some("perf"));
/// assert_eq!(branch_to_tag("hotfix/login-crash"),  Some("bugfix"));
/// assert_eq!(branch_to_tag("main"),                None);
/// assert_eq!(branch_to_tag("feature-no-slash"),    None);
/// ```
pub fn branch_to_tag(branch: &str) -> Option<&'static str> {
    let branch = branch.trim();

    // Default / trunk branches carry no task-type information.
    if matches!(branch, "main" | "master" | "develop" | "trunk" | "dev" | "") {
        return None;
    }

    // Extract the prefix before the first `/`.
    let prefix = branch.split('/').next()?;
    // Require a slash — a flat branch name (no prefix) is ambiguous.
    if !branch.contains('/') {
        return None;
    }

    match prefix {
        "feat" | "feature" | "features" => Some("feature"),
        "fix" | "bugfix" | "bug" | "hotfix" => Some("bugfix"),
        "refactor" | "refact" | "rfc" => Some("refactor"),
        "chore" | "build" | "ci" | "deps" => Some("chore"),
        "docs" | "doc" | "documentation" => Some("docs"),
        "test" | "tests" | "testing" => Some("test"),
        "perf" | "performance" | "optim" => Some("perf"),
        "release" | "rel" => Some("release"),
        "experiment" | "exp" | "spike" | "poc" => Some("experiment"),
        _ => None,
    }
}

/// Infer a task-type tag from observed tool-call patterns in a session.
///
/// TRIZ D8: Resolves NE-H (manual tagging friction). Pattern rules are applied
/// in priority order — first match wins. Complements `branch_to_tag()` for
/// sessions on feature-less branches or sessions without git context.
///
/// # Rules (in priority order)
///
/// | Pattern | Tag |
/// |---|---|
/// | ≥ 3 web-search / browser calls | `"research"` |
/// | ≥ 5 bash/exec calls with grep/find/rg | `"debugging"` |
/// | ≥ 3 write + ≥ 3 read calls (any file) | `"refactoring"` |
///
/// Returns `None` when no pattern matches (caller should fall back to `branch_to_tag`).
pub fn infer_tag_from_tool_calls(calls: &[ToolCall]) -> Option<&'static str> {
    let is_search = |tc: &ToolCall| {
        let n = tc.tool_name.to_lowercase();
        n.contains("search") || n.contains("browser") || n.contains("web") || n.contains("fetch")
    };
    let is_bash = |tc: &ToolCall| {
        let n = tc.tool_name.to_lowercase();
        n.contains("bash") || n.contains("exec") || n.contains("shell") || n.contains("run")
    };
    let is_grep_find = |tc: &ToolCall| {
        // Bash calls whose names suggest search/discovery.
        let n = tc.tool_name.to_lowercase();
        n.contains("grep")
            || n.contains("find")
            || n.contains("search")
            || n.contains("ripgrep")
            || n.contains("rg")
    };
    let is_write = |tc: &ToolCall| {
        let n = tc.tool_name.to_lowercase();
        n.contains("write") || n.contains("edit") || n.contains("create") || n.contains("patch")
    };
    let is_read = |tc: &ToolCall| {
        let n = tc.tool_name.to_lowercase();
        n.contains("read") || n.contains("view") || n.contains("cat") || n.contains("open")
    };

    let search_count = calls.iter().filter(|tc| is_search(tc)).count();
    if search_count >= 3 {
        return Some("research");
    }

    let bash_grep_count = calls
        .iter()
        .filter(|tc| is_bash(tc) || is_grep_find(tc))
        .count();
    if bash_grep_count >= 5 {
        return Some("debugging");
    }

    let write_count = calls.iter().filter(|tc| is_write(tc)).count();
    let read_count = calls.iter().filter(|tc| is_read(tc)).count();
    if write_count >= 3 && read_count >= 3 {
        return Some("refactoring");
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_common_prefixes() {
        assert_eq!(branch_to_tag("feat/auth"), Some("feature"));
        assert_eq!(branch_to_tag("feature/add-logging"), Some("feature"));
        assert_eq!(branch_to_tag("fix/cache-bug"), Some("bugfix"));
        assert_eq!(branch_to_tag("hotfix/login-crash"), Some("bugfix"));
        assert_eq!(branch_to_tag("refactor/db-layer"), Some("refactor"));
        assert_eq!(branch_to_tag("chore/update-deps"), Some("chore"));
        assert_eq!(branch_to_tag("docs/api-readme"), Some("docs"));
        assert_eq!(branch_to_tag("test/unit-suite"), Some("test"));
        assert_eq!(branch_to_tag("perf/faster-queries"), Some("perf"));
        assert_eq!(branch_to_tag("ci/add-audit"), Some("chore"));
        assert_eq!(branch_to_tag("release/v1.2.0"), Some("release"));
        assert_eq!(branch_to_tag("spike/websocket-poc"), Some("experiment"));
    }

    #[test]
    fn test_default_branches_return_none() {
        assert_eq!(branch_to_tag("main"), None);
        assert_eq!(branch_to_tag("master"), None);
        assert_eq!(branch_to_tag("develop"), None);
        assert_eq!(branch_to_tag("trunk"), None);
        assert_eq!(branch_to_tag(""), None);
    }

    #[test]
    fn test_flat_branch_name_returns_none() {
        assert_eq!(branch_to_tag("feature-no-slash"), None);
        assert_eq!(branch_to_tag("my-branch"), None);
    }

    #[test]
    fn test_unknown_prefix_returns_none() {
        assert_eq!(branch_to_tag("unknown/something"), None);
    }

    #[test]
    fn test_nested_paths_use_first_segment() {
        // Even deep paths like "feat/auth/oauth" → "feature"
        assert_eq!(branch_to_tag("feat/auth/oauth"), Some("feature"));
    }

    #[test]
    fn test_whitespace_trimmed() {
        assert_eq!(branch_to_tag("  feat/auth  "), Some("feature"));
    }

    // ── D8: infer_tag_from_tool_calls tests ──────────────────────────────────

    fn make_tool_call(name: &str) -> ToolCall {
        ToolCall {
            id: name.to_string(),
            turn_id: "t1".to_string(),
            session_id: "s1".to_string(),
            tool_name: name.to_string(),
            input_size_chars: 100,
            input_hash: 0,
            timestamp: 0,
        }
    }

    #[test]
    fn test_infer_research_tag() {
        let calls: Vec<ToolCall> = (0..4)
            .map(|i| make_tool_call(&format!("web_search_{i}")))
            .collect();
        assert_eq!(infer_tag_from_tool_calls(&calls), Some("research"));
    }

    #[test]
    fn test_infer_debugging_tag() {
        let calls: Vec<ToolCall> = (0..6)
            .map(|i| make_tool_call(&format!("bash_exec_{i}")))
            .collect();
        assert_eq!(infer_tag_from_tool_calls(&calls), Some("debugging"));
    }

    #[test]
    fn test_infer_refactoring_tag() {
        let mut calls: Vec<ToolCall> = (0..3)
            .map(|i| make_tool_call(&format!("write_file_{i}")))
            .collect();
        calls.extend((0..3).map(|i| make_tool_call(&format!("read_file_{i}"))));
        assert_eq!(infer_tag_from_tool_calls(&calls), Some("refactoring"));
    }

    #[test]
    fn test_infer_no_match_returns_none() {
        let calls: Vec<ToolCall> = (0..2)
            .map(|i| make_tool_call(&format!("some_tool_{i}")))
            .collect();
        assert_eq!(infer_tag_from_tool_calls(&calls), None);
    }
}
