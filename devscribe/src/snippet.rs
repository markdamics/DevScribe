//! Parses LSP snippet syntax (a completion item's `insert_text` when
//! `insert_text_format` is `InsertTextFormat::SNIPPET`) into plain
//! insertable text plus an ordered list of tab-stop ranges, so
//! `EditorState::begin_snippet` can select each placeholder in turn instead
//! of the raw `$1`/`${1:name}` syntax landing in the buffer literally.
//!
//! Only the subset actually seen from rust-analyzer/clangd/pyright/
//! typescript-language-server is handled: `$0`, `$1`, `${1}`, `${1:default}`,
//! and the three backslash escapes (`\$`, `\}`, `\\`) the spec defines for
//! literal text. Choice placeholders (`${1|a,b,c|}`), variables
//! (`$TM_SELECTED_TEXT`), and nested placeholders inside a default aren't —
//! any `$` that doesn't match one of the four numbered forms is copied
//! through literally rather than the whole snippet being rejected, same as
//! a plain-text completion. Two placeholders sharing the same number
//! ("mirrors", e.g. a getter/setter snippet using `$1` for the field name in
//! two places) aren't linked either — each is visited as its own stop in
//! `Tab` order rather than one edit updating both.

/// One placeholder's position in the *output* plain text — `stop` is its
/// LSP tab-stop number, `range` the half-open char range (into the returned
/// `ParsedSnippet::text`) that should be selected when a `Tab` walk reaches
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabStop {
    pub stop: u32,
    pub range: (usize, usize),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedSnippet {
    pub text: String,
    /// In visit order: every stop other than `$0` by ascending number, then
    /// `$0` (if present) last — per the spec, `$0` is always the final tab
    /// stop regardless of where it sits in the source text.
    pub tab_stops: Vec<TabStop>,
}

pub fn parse(input: &str) -> ParsedSnippet {
    let chars: Vec<char> = input.chars().collect();
    let mut text = String::with_capacity(chars.len());
    let mut out_len = 0usize;
    let mut stops: Vec<TabStop> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '$' && i + 1 < chars.len() {
            if chars[i + 1].is_ascii_digit() {
                let start = i + 1;
                let mut j = start;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    j += 1;
                }
                let num: u32 = chars[start..j].iter().collect::<String>().parse().unwrap_or(0);
                stops.push(TabStop { stop: num, range: (out_len, out_len) });
                i = j;
                continue;
            } else if chars[i + 1] == '{'
                && let Some((num, default, consumed)) = parse_braced(&chars[i + 2..])
            {
                let range_start = out_len;
                text.push_str(&default);
                out_len += default.chars().count();
                stops.push(TabStop { stop: num, range: (range_start, out_len) });
                i += 2 + consumed;
                continue;
            }
        }
        if c == '\\' && i + 1 < chars.len() && matches!(chars[i + 1], '$' | '}' | '\\') {
            text.push(chars[i + 1]);
            out_len += 1;
            i += 2;
            continue;
        }
        text.push(c);
        out_len += 1;
        i += 1;
    }
    stops.sort_by_key(|s| (s.stop == 0, s.stop));
    ParsedSnippet { text, tab_stops: stops }
}

/// Parses `<digits>[:default]}` — the caller has already consumed the
/// leading `${`. Returns `(stop_number, default_text, chars_consumed)`,
/// where `chars_consumed` includes the closing `}`. `None` if this isn't a
/// well-formed numbered placeholder (a choice list, a variable, ...); the
/// caller then falls back to copying the `$` through literally.
fn parse_braced(chars: &[char]) -> Option<(u32, String, usize)> {
    let mut j = 0;
    while j < chars.len() && chars[j].is_ascii_digit() {
        j += 1;
    }
    if j == 0 {
        return None;
    }
    let num: u32 = chars[..j].iter().collect::<String>().parse().ok()?;
    if chars.get(j) == Some(&'}') {
        return Some((num, String::new(), j + 1));
    }
    if chars.get(j) == Some(&':') {
        let default_start = j + 1;
        let mut k = default_start;
        while k < chars.len() && chars[k] != '}' {
            k += 1;
        }
        if k < chars.len() {
            let default: String = chars[default_start..k].iter().collect();
            return Some((num, default, k + 1));
        }
    }
    None
}

#[cfg(test)]
#[path = "tests/snippet.rs"]
mod tests;
