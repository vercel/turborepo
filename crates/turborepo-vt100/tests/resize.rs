use turborepo_vt100 as vt100;

#[test]
fn resize_cols_shrink() {
    let mut parser = vt100::Parser::new(3, 6, 0);
    parser.process(b"foobarbaz");
    assert_contents(&parser, "foobar\nbaz\n\n");
    assert!(
        parser.screen().row_wrapped(0),
        "first row should be wrapped"
    );
    parser.screen_mut().set_size(3, 3);
    assert_contents(&parser, "foo\nbar\nbaz\n");
    assert!(
        parser.screen().row_wrapped(0),
        "first row should be wrapped"
    );
    assert!(
        parser.screen().row_wrapped(1),
        "second row should be wrapped"
    );
    assert!(
        !parser.screen().row_wrapped(2),
        "final row should not be wrapped"
    );
    parser.screen_mut().set_size(3, 6);
    assert_contents(&parser, "foobar\nbaz\n\n");
}

#[test]
fn resize_cols_shrink_whitespace() {
    let mut parser = vt100::Parser::new(3, 10, 0);
    parser.process(b"foo bar");
    assert_contents(&parser, "foo bar\n\n\n");
    parser.screen_mut().set_size(3, 8);
    assert_contents(&parser, "foo bar\n\n\n");
    parser.screen_mut().set_size(3, 6);
    assert_contents(&parser, "foo ba\nr\n\n");
    parser.screen_mut().set_size(3, 4);
    assert_contents(&parser, "foo \nbar\n\n");
    parser.screen_mut().set_size(3, 3);
    assert_contents(&parser, "foo\n ba\nr\n");
}

#[test]
fn resize_cols_expand() {
    let mut parser = vt100::Parser::new(3, 3, 0);
    parser.process(b"foobarbaz");
    assert_contents(&parser, "foo\nbar\nbaz\n");
    parser.screen_mut().set_size(3, 6);
    assert_contents(&parser, "foobar\nbaz\n\n");
    parser.screen_mut().set_size(3, 9);
    assert_contents(&parser, "foobarbaz\n\n\n");
}

#[test]
fn cols_expand_preserves_newlines() {
    let mut parser = vt100::Parser::new(4, 6, 0);
    parser.process(b"foobar\r\n\r\nbaz");
    assert_contents(&parser, "foobar\n\nbaz\n\n");
    parser.screen_mut().set_size(4, 3);
    assert_contents(&parser, "foo\nbar\n\nbaz\n");
    parser.screen_mut().set_size(4, 6);
    assert_contents(&parser, "foobar\n\nbaz\n\n");
}

#[test]
fn cols_expand_preserves_newlines_multiline_wrap() {
    let mut parser = vt100::Parser::new(5, 6, 0);
    parser.process(b"foobar\r\n\r\nbaz");
    assert_contents(&parser, "foobar\n\nbaz\n\n\n");
    parser.screen_mut().set_size(4, 2);
    assert_contents(&parser, "fo\nob\nar\n\nbaz\n");
    parser.screen_mut().set_size(4, 6);
    assert_contents(&parser, "foobar\n\nbaz\n\n\n");
}

#[test]
fn test_resize_with_scrollback() {
    // we need to test that scrollback gets resized as well
    // also gotta figure out if we're in a scrolled position what do do with new rows that don't fit
    todo!()
}

fn assert_contents(parser: &vt100::Parser, expected: &str) {
    let mut contents = String::new();
    let screen = parser.screen();
    let (rows, cols) = screen.size();
    for i in 0..rows {
        for j in 0..cols {
            if let Some(cell) = screen.cell(i, j) {
                contents.push_str(&cell.contents());
            }
        }
        contents.push('\n');
    }

    assert_eq!(contents, expected);
}
