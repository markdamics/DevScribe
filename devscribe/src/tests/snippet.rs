use super::*;

#[test]
fn plain_text_with_no_dollar_signs_passes_through_with_no_stops() {
    let parsed = parse("hello world");
    assert_eq!(parsed.text, "hello world");
    assert!(parsed.tab_stops.is_empty());
}

#[test]
fn a_bare_numbered_stop_is_zero_width_at_its_position() {
    let parsed = parse("foo($1)");
    assert_eq!(parsed.text, "foo()");
    assert_eq!(parsed.tab_stops, vec![TabStop { stop: 1, range: (4, 4) }]);
}

#[test]
fn a_braced_stop_with_no_default_is_zero_width() {
    let parsed = parse("foo(${1})");
    assert_eq!(parsed.text, "foo()");
    assert_eq!(parsed.tab_stops, vec![TabStop { stop: 1, range: (4, 4) }]);
}

#[test]
fn a_braced_stop_with_a_default_selects_the_default_text() {
    let parsed = parse("fn ${1:name}()");
    assert_eq!(parsed.text, "fn name()");
    let stop = parsed.tab_stops[0];
    assert_eq!(stop.stop, 1);
    assert_eq!(&parsed.text[stop.range.0..stop.range.1], "name");
}

#[test]
fn a_default_containing_a_colon_is_kept_whole() {
    let parsed = parse("${1:foo:bar}");
    assert_eq!(parsed.text, "foo:bar");
    assert_eq!(parsed.tab_stops[0].range, (0, 7));
}

#[test]
fn a_realistic_function_snippet_orders_dollar_zero_last() {
    let parsed = parse("fn ${1:name}() {\n    $0\n}");
    assert_eq!(parsed.text, "fn name() {\n    \n}");
    assert_eq!(parsed.tab_stops.len(), 2);
    assert_eq!(parsed.tab_stops[0].stop, 1, "the named placeholder is visited first");
    assert_eq!(&parsed.text[parsed.tab_stops[0].range.0..parsed.tab_stops[0].range.1], "name");
    assert_eq!(parsed.tab_stops[1].stop, 0, "$0 is always visited last");
    assert_eq!(parsed.tab_stops[1].range.0, parsed.tab_stops[1].range.1, "$0 has no default text");
}

#[test]
fn stops_are_ordered_by_number_regardless_of_source_order_with_zero_always_last() {
    let parsed = parse("${2:b}${1:a}$0");
    let order: Vec<u32> = parsed.tab_stops.iter().map(|s| s.stop).collect();
    assert_eq!(order, vec![1, 2, 0]);
}

#[test]
fn backslash_escapes_are_unescaped_and_not_mistaken_for_placeholders() {
    let parsed = parse(r"\$5\}\\");
    assert_eq!(parsed.text, "$5}\\");
    assert!(parsed.tab_stops.is_empty());
}

#[test]
fn an_unrecognized_brace_form_is_copied_through_literally() {
    // A choice placeholder (`${1|a,b|}`) isn't one of the forms this parser
    // understands — it must not be dropped or mis-parsed, just left as-is.
    let parsed = parse("${1|a,b|}");
    assert_eq!(parsed.text, "${1|a,b|}");
    assert!(parsed.tab_stops.is_empty());
}

#[test]
fn mirrored_stops_sharing_a_number_are_kept_as_separate_visits() {
    // Documented limitation: two placeholders with the same number aren't
    // linked, so both still show up as their own stop rather than being
    // merged or dropped.
    let parsed = parse("${1:x} = ${1:x}");
    assert_eq!(parsed.tab_stops.len(), 2);
    assert!(parsed.tab_stops.iter().all(|s| s.stop == 1));
}
