use turborepo_vt100 as vt100;

#[test]
fn test_screen_includes_scrollback() {
    let mut parser = vt100::Parser::new(2, 20, 100);
    parser.process(b"foo\r\nbar\r\nbaz\r\n");
    let screen = parser.entire_screen();
    assert_eq!(screen.contents(), "foo\nbar\nbaz");
    assert_eq!(screen.size(), (3, 20));
}

#[test]
fn test_screen_trims_trailing_blank_lines() {
    let mut parser = vt100::Parser::new(8, 20, 0);
    parser.process(b"foo\r\nbar\r\n");
    let screen = parser.entire_screen();
    assert_eq!(screen.contents(), "foo\nbar");
    assert_eq!(screen.size(), (2, 20));
}

#[test]
fn test_wrapped_lines_size() {
    let mut parser = vt100::Parser::new(8, 8, 10);
    parser.process(b"one long line\r\nbar\r\n");
    let screen = parser.entire_screen();
    assert_eq!(screen.contents(), "one long line\nbar");
    assert_eq!(screen.size(), (3, 8));
    assert_eq!(screen.cell(0, 0).unwrap().contents(), "o");
    assert_eq!(screen.cell(1, 0).unwrap().contents(), " ");
    // "one long line"
    //         ^ last char that fits on line, rest will appear on next row
    assert_eq!(screen.cell(2, 0).unwrap().contents(), "b");
}
