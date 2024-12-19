mod helpers;

#[test]
fn split_escape_sequences() {
    helpers::fixture("split_escape_sequences");
}

#[test]
fn split_utf8() {
    helpers::fixture("split_utf8");
}
