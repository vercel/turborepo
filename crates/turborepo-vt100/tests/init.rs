use turborepo_vt100 as vt100;

#[test]
fn init() {
    let parser = vt100::Parser::default();
    assert_eq!(parser.screen().size(), (24, 80));
    assert_eq!(parser.screen().cursor_position(), (0, 0));

    let cell = parser.screen().cell(0, 0);
    assert_eq!(cell.unwrap().contents(), "");
    let cell = parser.screen().cell(23, 79);
    assert_eq!(cell.unwrap().contents(), "");
    let cell = parser.screen().cell(24, 0);
    assert!(cell.is_none());
    let cell = parser.screen().cell(0, 80);
    assert!(cell.is_none());

    assert_eq!(parser.screen().contents(), "");
    assert_eq!(
        parser.screen().contents_formatted(),
        b"\x1b[?25h\x1b[m\x1b[H\x1b[J"
    );

    assert_eq!(parser.screen().title(), "");
    assert_eq!(parser.screen().icon_name(), "");

    assert!(!parser.screen().application_keypad());
    assert!(!parser.screen().application_cursor());
    assert!(!parser.screen().hide_cursor());
    assert!(!parser.screen().bracketed_paste());
    assert_eq!(
        parser.screen().mouse_protocol_mode(),
        vt100::MouseProtocolMode::None
    );
    assert_eq!(
        parser.screen().mouse_protocol_encoding(),
        vt100::MouseProtocolEncoding::Default
    );
}
