mod helpers;

#[test]
fn colors() {
    helpers::fixture("colors");
}

#[test]
fn attrs() {
    helpers::fixture("attrs");
}

#[test]
fn attributes_formatted() {
    let mut parser = turborepo_vt100::Parser::default();
    assert_eq!(parser.screen().attributes_formatted(), b"\x1b[m");
    parser.process(b"\x1b[32mfoo\x1b[41mbar\x1b[33mbaz");
    assert_eq!(parser.screen().attributes_formatted(), b"\x1b[m\x1b[33;41m");
    parser.process(b"\x1b[1m\x1b[39m");
    assert_eq!(parser.screen().attributes_formatted(), b"\x1b[m\x1b[41;1m");
    parser.process(b"\x1b[m");
    assert_eq!(parser.screen().attributes_formatted(), b"\x1b[m");
}
