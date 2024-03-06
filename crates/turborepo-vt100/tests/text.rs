mod helpers;

#[test]
fn ascii() {
    helpers::fixture("ascii");
}

#[test]
fn utf8() {
    helpers::fixture("utf8");
}

#[test]
fn newlines() {
    helpers::fixture("newlines");
}

#[test]
fn wide() {
    helpers::fixture("wide");
}

#[test]
fn combining() {
    helpers::fixture("combining");
}

#[test]
fn wrap() {
    helpers::fixture("wrap");
}

#[test]
fn wrap_weird() {
    helpers::fixture("wrap_weird");
}
