//! A minimal case-insensitive subsequence fuzzy matcher, used to narrow the
//! completion popup as the user types past the trigger character — see
//! `state::editor::EditorState::refilter_completions`. Not a general-purpose
//! "fzf-style" ranker: just enough to reward a match starting at the
//! haystack's own first character and runs of consecutive matched
//! characters, so typing a prefix like `"clo"` ranks `"clone"` above
//! `"reclosest"` instead of however the language server happened to order
//! same-length alternatives.

/// `None` if `needle`'s characters don't all appear in `haystack`, in order
/// (not necessarily contiguously); otherwise `Some(score)`, higher meaning a
/// better match. An empty `needle` matches everything with a score of `0` —
/// the "nothing typed yet" case, where the caller wants the server's own
/// order preserved rather than every item tying at the same score.
pub fn score(needle: &str, haystack: &str) -> Option<i32> {
    if needle.is_empty() {
        return Some(0);
    }

    let needle: Vec<char> = needle.chars().flat_map(char::to_lowercase).collect();
    let haystack: Vec<char> = haystack.chars().flat_map(char::to_lowercase).collect();

    let mut score = 0i32;
    let mut search_from = 0usize;
    let mut prev_matched_at: Option<usize> = None;
    for (i, &nc) in needle.iter().enumerate() {
        let idx = search_from + haystack[search_from..].iter().position(|&hc| hc == nc)?;
        score += 10;
        if i == 0 && idx == 0 {
            // Rewards a match that starts right at the identifier's own
            // first character — the common case of typing an actual prefix.
            score += 15;
        }
        if prev_matched_at == Some(idx.wrapping_sub(1)) {
            score += 8;
        }
        prev_matched_at = Some(idx);
        search_from = idx + 1;
    }
    Some(score)
}

#[cfg(test)]
#[path = "tests/fuzzy.rs"]
mod tests;
