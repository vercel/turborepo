use turborepo_vt100 as vt100;

mod helpers;

#[test]
fn absolute_movement() {
    helpers::fixture("absolute_movement");
}

#[test]
fn row_clamp() {
    let mut vt = vt100::Parser::default();
    assert_eq!(vt.screen().cursor_position(), (0, 0));
    vt.process(b"\x1b[15d");
    assert_eq!(vt.screen().cursor_position(), (14, 0));
    vt.process(b"\x1b[150d");
    assert_eq!(vt.screen().cursor_position(), (23, 0));
}

#[test]
fn relative_movement() {
    helpers::fixture("relative_movement");
}

#[test]
fn ed() {
    helpers::fixture("ed");
}

#[test]
fn el() {
    helpers::fixture("el");
}

#[test]
fn ich_dch_ech() {
    helpers::fixture("ich_dch_ech");
}

#[test]
fn il_dl() {
    helpers::fixture("il_dl");
}

#[test]
fn scroll() {
    helpers::fixture("scroll");
}

#[test]
fn xtwinops() {
    struct Callbacks;
    impl vt100::Callbacks for Callbacks {
        fn resize(
            &mut self,
            screen: &mut vt100::Screen,
            (rows, cols): (u16, u16),
        ) {
            screen.set_size(rows, cols);
        }
    }

    let mut vt = vt100::Parser::default();
    assert_eq!(vt.screen().size(), (24, 80));
    vt.process_cb(b"\x1b[8;24;80t", &mut Callbacks);
    assert_eq!(vt.screen().size(), (24, 80));
    vt.process_cb(b"\x1b[8t", &mut Callbacks);
    assert_eq!(vt.screen().size(), (24, 80));
    vt.process_cb(b"\x1b[8;80;24t", &mut Callbacks);
    assert_eq!(vt.screen().size(), (80, 24));
    vt.process_cb(b"\x1b[8;24t", &mut Callbacks);
    assert_eq!(vt.screen().size(), (24, 24));

    let mut vt = vt100::Parser::default();
    assert_eq!(vt.screen().size(), (24, 80));
    vt.process_cb(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", &mut Callbacks);
    assert_eq!(
        vt.screen().rows(0, 80).next().unwrap(),
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
    assert_eq!(vt.screen().rows(0, 80).nth(1).unwrap(), "aaaaaaaaaa");
    vt.process_cb(
        b"\x1b[H\x1b[8;24;15tbbbbbbbbbbbbbbbbbbbb\x1b[8;24;80tcccccccccccccccccccc",
        &mut Callbacks,
    );
    assert_eq!(vt.screen().rows(0, 80).next().unwrap(), "bbbbbbbbbbbbbbb");
    assert_eq!(
        vt.screen().rows(0, 80).nth(1).unwrap(),
        "bbbbbcccccccccccccccccccc"
    );
}
