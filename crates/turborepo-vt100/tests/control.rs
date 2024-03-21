mod helpers;

use turborepo_vt100 as vt100;

#[test]
fn bel() {
    struct State {
        bel: usize,
    }

    impl vt100::Callbacks for State {
        fn audible_bell(&mut self, _: &mut vt100::Screen) {
            self.bel += 1;
        }
    }

    let mut parser = vt100::Parser::default();
    let mut state = State { bel: 0 };
    assert_eq!(state.bel, 0);

    let screen = parser.screen().clone();
    parser.process_cb(b"\x07", &mut state);
    assert_eq!(state.bel, 1);
    assert_eq!(parser.screen().contents_diff(&screen), b"");

    let screen = parser.screen().clone();
    parser.process_cb(b"\x07", &mut state);
    assert_eq!(state.bel, 2);
    assert_eq!(parser.screen().contents_diff(&screen), b"");

    let screen = parser.screen().clone();
    parser.process_cb(b"\x07\x07\x07", &mut state);
    assert_eq!(state.bel, 5);
    assert_eq!(parser.screen().contents_diff(&screen), b"");

    let screen = parser.screen().clone();
    parser.process_cb(b"foo", &mut state);
    assert_eq!(state.bel, 5);
    assert_eq!(parser.screen().contents_diff(&screen), b"foo");

    let screen = parser.screen().clone();
    parser.process_cb(b"ba\x07r", &mut state);
    assert_eq!(state.bel, 6);
    assert_eq!(parser.screen().contents_diff(&screen), b"bar");
}

#[test]
fn bs() {
    helpers::fixture("bs");
}

#[test]
fn tab() {
    helpers::fixture("tab");
}

#[test]
fn lf() {
    helpers::fixture("lf");
}

#[test]
fn vt() {
    helpers::fixture("vt");
}

#[test]
fn ff() {
    helpers::fixture("ff");
}

#[test]
fn cr() {
    helpers::fixture("cr");
}
