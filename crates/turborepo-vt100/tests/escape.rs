use turborepo_vt100 as vt100;

mod helpers;

#[test]
fn deckpam() {
    helpers::fixture("deckpam");
}

#[test]
fn ri() {
    helpers::fixture("ri");
}

#[test]
fn ris() {
    helpers::fixture("ris");
}

#[test]
fn vb() {
    struct State {
        vb: usize,
    }

    impl vt100::Callbacks for State {
        fn visual_bell(&mut self, _: &mut vt100::Screen) {
            self.vb += 1;
        }
    }

    let mut parser = vt100::Parser::default();
    let mut state = State { vb: 0 };
    assert_eq!(state.vb, 0);

    let screen = parser.screen().clone();
    parser.process_cb(b"\x1bg", &mut state);
    assert_eq!(state.vb, 1);
    assert_eq!(parser.screen().contents_diff(&screen), b"");

    let screen = parser.screen().clone();
    parser.process_cb(b"\x1bg", &mut state);
    assert_eq!(state.vb, 2);
    assert_eq!(parser.screen().contents_diff(&screen), b"");

    let screen = parser.screen().clone();
    parser.process_cb(b"\x1bg\x1bg\x1bg", &mut state);
    assert_eq!(state.vb, 5);
    assert_eq!(parser.screen().contents_diff(&screen), b"");

    let screen = parser.screen().clone();
    parser.process_cb(b"foo", &mut state);
    assert_eq!(state.vb, 5);
    assert_eq!(parser.screen().contents_diff(&screen), b"foo");

    let screen = parser.screen().clone();
    parser.process_cb(b"ba\x1bgr", &mut state);
    assert_eq!(state.vb, 6);
    assert_eq!(parser.screen().contents_diff(&screen), b"bar");
}

#[test]
fn decsc() {
    helpers::fixture("decsc");
}
