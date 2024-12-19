use turborepo_vt100 as vt100;

use std::io::Write as _;

#[test]
fn write_text() {
    let mut parser = vt100::Parser::default();
    let input = b"foo\x1b[31m\x1b[32mb\x1b[3;7;42ma\x1b[23mr";
    let bytes = parser.write(input).unwrap();
    assert_eq!(bytes, input.len());
    assert_eq!(parser.screen().contents(), "foobar");
}

#[test]
fn cell_contents() {
    let mut parser = vt100::Parser::default();
    let input = b"foo\x1b[31m\x1b[32mb\x1b[3;7;42ma\x1b[23mr";
    let bytes = parser.write(input).unwrap();
    assert_eq!(bytes, input.len());
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
    let bytes = parser.write(input).unwrap();
    assert_eq!(bytes, input.len());

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
    let bytes = parser.write(input).unwrap();
    assert_eq!(bytes, input.len());

    assert!(parser.screen().cell(0, 4).unwrap().italic());
}
