use turborepo_vt100 as vt100;

#[test]
fn object_creation() {
    let parser = vt100::Parser::default();
    assert_eq!(parser.screen().size(), (24, 80));
}

#[test]
fn process_text() {
    let mut parser = vt100::Parser::default();
    let input = b"foo\x1b[31m\x1b[32mb\x1b[3;7;42ma\x1b[23mr";
    parser.process(input);
    assert_eq!(parser.screen().contents(), "foobar");
}

#[test]
fn set_size() {
    let mut parser = vt100::Parser::default();
    assert_eq!(parser.screen().size(), (24, 80));
    assert_eq!(parser.screen().cursor_position(), (0, 0));

    parser.screen_mut().set_size(34, 8);
    assert_eq!(parser.screen().size(), (34, 8));
    assert_eq!(parser.screen().cursor_position(), (0, 0));

    parser.process(b"\x1b[30;5H");
    assert_eq!(parser.screen().cursor_position(), (29, 4));

    parser.screen_mut().set_size(24, 80);
    assert_eq!(parser.screen().size(), (24, 80));
    assert_eq!(parser.screen().cursor_position(), (23, 4));

    parser.screen_mut().set_size(34, 8);
    assert_eq!(parser.screen().size(), (34, 8));
    assert_eq!(parser.screen().cursor_position(), (23, 4));

    parser.process(b"\x1b[?1049h");
    assert_eq!(parser.screen().size(), (34, 8));
    assert_eq!(parser.screen().cursor_position(), (0, 0));

    parser.screen_mut().set_size(24, 80);
    assert_eq!(parser.screen().size(), (24, 80));
    assert_eq!(parser.screen().cursor_position(), (0, 0));

    parser.process(b"\x1b[?1049l");
    assert_eq!(parser.screen().size(), (24, 80));
    assert_eq!(parser.screen().cursor_position(), (23, 4));

    parser.screen_mut().set_size(34, 8);
    parser.process(b"\x1bc01234567890123456789");
    assert_eq!(parser.screen().contents(), "01234567890123456789");

    parser.screen_mut().set_size(24, 80);
    assert_eq!(parser.screen().contents(), "01234567\n89012345\n6789");

    parser.screen_mut().set_size(34, 8);
    assert_eq!(parser.screen().contents(), "01234567\n89012345\n6789");

    let mut parser = vt100::Parser::default();
    assert_eq!(parser.screen().size(), (24, 80));
    parser.screen_mut().set_size(30, 100);
    assert_eq!(parser.screen().size(), (30, 100));
    parser.process(b"\x1b[75Cfoobar");
    assert_eq!(
        parser.screen().contents(),
        "                                                                           foobar"
    );

    let mut parser = vt100::Parser::default();
    assert_eq!(parser.screen().size(), (24, 80));
    parser.screen_mut().set_size(30, 100);
    assert_eq!(parser.screen().size(), (30, 100));
    parser.process(b"1\r\n2\r\n3\r\n4\r\n5\r\n6\r\n7\r\n8\r\n9\r\n10\r\n11\r\n12\r\n13\r\n14\r\n15\r\n16\r\n17\r\n18\r\n19\r\n20\r\n21\r\n22\r\n23\r\n24\x1b[24;99Hfoobar");
    assert_eq!(
        parser.screen().contents(),
        "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\n14\n15\n16\n17\n18\n19\n20\n21\n22\n23\n24                                                                                                foobar"
    );
}

#[test]
fn cell_contents() {
    let mut parser = vt100::Parser::default();
    let input = b"foo\x1b[31m\x1b[32mb\x1b[3;7;42ma\x1b[23mr";
    parser.process(input);
    assert_eq!(parser.screen().cell(0, 0).unwrap().contents(), "f");
    assert_eq!(parser.screen().cell(0, 1).unwrap().contents(), "o");
    assert_eq!(parser.screen().cell(0, 2).unwrap().contents(), "o");
    assert_eq!(parser.screen().cell(0, 3).unwrap().contents(), "b");
    assert_eq!(parser.screen().cell(0, 4).unwrap().contents(), "a");
    assert_eq!(parser.screen().cell(0, 5).unwrap().contents(), "r");
    assert_eq!(parser.screen().cell(0, 6).unwrap().contents(), "");
}

#[test]
fn cell_colors() {
    let mut parser = vt100::Parser::default();
    let input = b"foo\x1b[31m\x1b[32mb\x1b[3;7;42ma\x1b[23mr";
    parser.process(input);

    assert_eq!(
        parser.screen().cell(0, 0).unwrap().fgcolor(),
        vt100::Color::Default
    );
    assert_eq!(
        parser.screen().cell(0, 3).unwrap().fgcolor(),
        vt100::Color::Idx(2)
    );
    assert_eq!(
        parser.screen().cell(0, 4).unwrap().fgcolor(),
        vt100::Color::Idx(2)
    );
    assert_eq!(
        parser.screen().cell(0, 4).unwrap().bgcolor(),
        vt100::Color::Idx(2)
    );
}

#[test]
fn cell_attrs() {
    let mut parser = vt100::Parser::default();
    let input = b"foo\x1b[31m\x1b[32mb\x1b[3;7;42ma\x1b[23mr";
    parser.process(input);

    assert!(parser.screen().cell(0, 4).unwrap().italic());
}
