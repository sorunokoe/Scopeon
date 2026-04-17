/// Truncate to at most `max_chars` Unicode scalar values (no ellipsis).
pub fn truncate_to_chars(input: &str, max_chars: usize) -> String {
    input.chars().take(max_chars).collect()
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
