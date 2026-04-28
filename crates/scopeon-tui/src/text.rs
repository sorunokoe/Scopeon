/// Truncate to at most `max_chars` Unicode scalar values (no ellipsis).
pub fn truncate_to_chars(input: &str, max_chars: usize) -> String {
    input.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── truncate_to_chars ────────────────────────────────────────────────────

    #[test]
    fn truncate_to_chars_shorter_than_limit_unchanged() {
        assert_eq!(truncate_to_chars("hello", 10), "hello");
    }

    #[test]
    fn truncate_to_chars_exactly_at_limit_unchanged() {
        assert_eq!(truncate_to_chars("hello", 5), "hello");
    }

    #[test]
    fn truncate_to_chars_over_limit_cuts_at_boundary() {
        let result = truncate_to_chars("hello world", 5);
        assert_eq!(result, "hello");
        assert_eq!(result.chars().count(), 5);
    }

    #[test]
    fn truncate_to_chars_zero_limit_returns_empty() {
        assert_eq!(truncate_to_chars("hello", 0), "");
    }

    #[test]
    fn truncate_to_chars_empty_input_returns_empty() {
        assert_eq!(truncate_to_chars("", 10), "");
    }

    #[test]
    fn truncate_to_chars_multibyte_unicode_counts_by_char_not_byte() {
        // "日本語" = 3 chars, 9 bytes
        let result = truncate_to_chars("日本語テスト", 3);
        assert_eq!(result, "日本語");
        assert_eq!(result.chars().count(), 3);
        // Must not produce invalid UTF-8 by slicing mid-byte
        assert!(result.is_ascii() || !result.is_empty());
    }

    #[test]
    fn truncate_to_chars_emoji_counts_as_single_char() {
        // 🚀 = 1 char, 4 bytes
        let result = truncate_to_chars("🚀🎯🔥", 2);
        assert_eq!(result.chars().count(), 2);
        assert_eq!(result, "🚀🎯");
    }

    // ── truncate_with_ellipsis ───────────────────────────────────────────────

    #[test]
    fn ellipsis_shorter_than_limit_unchanged() {
        assert_eq!(truncate_with_ellipsis("hello", 10), "hello");
    }

    #[test]
    fn ellipsis_exactly_at_limit_unchanged_no_ellipsis() {
        let result = truncate_with_ellipsis("hello", 5);
        assert_eq!(result, "hello");
        assert!(!result.contains('…'), "no ellipsis when exactly at limit");
    }

    #[test]
    fn ellipsis_one_over_limit_appends_ellipsis() {
        // "hello!" is 6 chars, limit 5 → "hell…"
        let result = truncate_with_ellipsis("hello!", 5);
        assert_eq!(result.chars().count(), 5);
        assert!(result.ends_with('…'), "must end with ellipsis");
        assert_eq!(result, "hell…");
    }

    #[test]
    fn ellipsis_far_over_limit_result_is_exactly_limit_chars() {
        let long_str = "a".repeat(100);
        let result = truncate_with_ellipsis(&long_str, 20);
        assert_eq!(result.chars().count(), 20, "result must be exactly max_chars wide");
        assert!(result.ends_with('…'));
    }

    #[test]
    fn ellipsis_zero_limit_returns_empty() {
        assert_eq!(truncate_with_ellipsis("hello", 0), "");
    }

    #[test]
    fn ellipsis_limit_one_returns_single_ellipsis() {
        // limit=1: truncate to 0 chars + '…' → just '…'
        let result = truncate_with_ellipsis("hello", 1);
        assert_eq!(result.chars().count(), 1);
        assert_eq!(result, "…");
    }

    #[test]
    fn ellipsis_empty_input_returns_empty() {
        assert_eq!(truncate_with_ellipsis("", 10), "");
    }

    #[test]
    fn ellipsis_multibyte_unicode_counts_chars_not_bytes() {
        // "日本語テスト" = 6 chars; limit 4 → "日本語…"
        let result = truncate_with_ellipsis("日本語テスト", 4);
        assert_eq!(result.chars().count(), 4);
        assert!(result.ends_with('…'));
        assert_eq!(result, "日本語…");
    }

    #[test]
    fn ellipsis_result_never_empty_for_nonempty_input_when_limit_positive() {
        for limit in 1..=5 {
            let result = truncate_with_ellipsis("something long enough", limit);
            assert!(!result.is_empty(), "limit={limit} must not produce empty string");
            assert_eq!(result.chars().count(), limit);
        }
    }

    #[test]
    fn ellipsis_ascii_provider_name_fits_chip_width() {
        // Provider names in chip_row have width ~20; must never be cut mid-display
        let name = "github-copilot-cli-provider";
        let result = truncate_with_ellipsis(name, 20);
        assert_eq!(result.chars().count(), 20);
        assert!(result.ends_with('…'));
        // First 19 chars come from the original
        let expected_prefix: String = name.chars().take(19).collect();
        assert!(result.starts_with(&expected_prefix));
    }
}

/// Truncate to at most `max_chars` Unicode scalar values, appending an ellipsis
/// when truncation occurs.
///
/// Behavior:
/// - if input length <= `max_chars`: returns input unchanged
/// - if input length > `max_chars`: returns first `max_chars - 1` chars + `…`
pub fn truncate_with_ellipsis(input: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }

    let mut iter = input.chars();
    let mut prefix = String::new();

    for _ in 0..max_chars {
        match iter.next() {
            Some(ch) => prefix.push(ch),
            None => return prefix,
        }
    }

    if iter.next().is_none() {
        return prefix;
    }

    let mut out: String = prefix.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}
