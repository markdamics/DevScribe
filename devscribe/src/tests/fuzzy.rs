use super::*;

#[test]
fn empty_needle_matches_everything_with_a_zero_score() {
    assert_eq!(score("", "anything"), Some(0));
    assert_eq!(score("", ""), Some(0));
}

#[test]
fn every_needle_char_must_appear_in_order() {
    assert!(score("cln", "clone").is_some());
    assert!(score("nlc", "clone").is_none(), "wrong order must not match");
    assert!(score("clonex", "clone").is_none(), "needle longer than any match must not match");
}

#[test]
fn matching_is_case_insensitive() {
    assert!(score("CLO", "clone").is_some());
    assert!(score("clo", "CLONE").is_some());
}

#[test]
fn a_prefix_match_outscores_a_scattered_subsequence_match() {
    let prefix = score("clo", "clone").unwrap();
    let scattered = score("clo", "cancel_lookup_operation").unwrap();
    assert!(prefix > scattered, "prefix={prefix} scattered={scattered}");
}

#[test]
fn consecutive_matches_outscore_the_same_characters_spread_apart() {
    let consecutive = score("ab", "ab").unwrap();
    let spread = score("ab", "a_b").unwrap();
    assert!(consecutive > spread, "consecutive={consecutive} spread={spread}");
}

#[test]
fn a_non_matching_needle_is_none() {
    assert_eq!(score("xyz", "clone"), None);
}
