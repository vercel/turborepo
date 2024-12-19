use turborepo_vt100 as vt100;

mod helpers;

#[test]
fn intermediate_control() {
    helpers::fixture("intermediate_control");
}

#[test]
fn params() {
    let mut parser = vt100::Parser::default();
    parser.process(b"\x1b[::::::::::::::::::::::::::::::::@");
    parser.process(b"\x1b[::::::::::::::::::::::::::::::::H");
    parser.process(b"\x1b[::::::::::::::::::::::::::::::::r");
    parser.process(b"a\x1b[8888888X");
}
