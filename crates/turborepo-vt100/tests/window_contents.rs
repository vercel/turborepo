#![allow(unused_imports)]
use turborepo_vt100 as vt100;

mod helpers;

use std::io::Read as _;

#[test]
fn formatted() {
    let mut parser = vt100::Parser::default();
    helpers::contents_formatted_reproduces_screen(parser.screen());
    assert_eq!(
        parser.screen().contents_formatted(),
        b"\x1b[?25h\x1b[m\x1b[H\x1b[J"
    );

    parser.process(b"foobar");
    helpers::contents_formatted_reproduces_screen(parser.screen());
    assert!(!parser.screen().cell(0, 2).unwrap().bold());
    assert!(!parser.screen().cell(0, 3).unwrap().bold());
    assert!(!parser.screen().cell(0, 4).unwrap().bold());
    assert!(!parser.screen().cell(0, 5).unwrap().bold());
    assert_eq!(
        parser.screen().contents_formatted(),
        b"\x1b[?25h\x1b[m\x1b[H\x1b[Jfoobar"
    );

    parser.process(b"\x1b[1;4H\x1b[1;7m\x1b[33mb");
    helpers::contents_formatted_reproduces_screen(parser.screen());
    assert!(!parser.screen().cell(0, 2).unwrap().bold());
    assert!(parser.screen().cell(0, 3).unwrap().bold());
    assert!(!parser.screen().cell(0, 4).unwrap().bold());
    assert!(!parser.screen().cell(0, 5).unwrap().bold());
    assert_eq!(
        parser.screen().contents_formatted(),
        &b"\x1b[?25h\x1b[m\x1b[H\x1b[Jfoo\x1b[33;1;7mb\x1b[mar\x1b[1;5H\x1b[33;1;7m"[..]
    );

    parser.process(b"\x1b[1;5H\x1b[22;42ma");
    helpers::contents_formatted_reproduces_screen(parser.screen());
    assert!(!parser.screen().cell(0, 2).unwrap().bold());
    assert!(parser.screen().cell(0, 3).unwrap().bold());
    assert!(!parser.screen().cell(0, 4).unwrap().bold());
    assert!(!parser.screen().cell(0, 5).unwrap().bold());
    assert_eq!(
        parser.screen().contents_formatted(),
        &b"\x1b[?25h\x1b[m\x1b[H\x1b[Jfoo\x1b[33;1;7mb\x1b[42;22ma\x1b[mr\x1b[1;6H\x1b[33;42;7m"
            [..]
    );

    parser.process(b"\x1b[1;6H\x1b[35mr\r\nquux");
    helpers::contents_formatted_reproduces_screen(parser.screen());
    assert_eq!(
        parser.screen().contents_formatted(),
        &b"\x1b[?25h\x1b[m\x1b[H\x1b[Jfoo\x1b[33;1;7mb\x1b[42;22ma\x1b[35mr\r\nquux"[..]
    );

    parser.process(b"\x1b[2;1H\x1b[45mquux");
    helpers::contents_formatted_reproduces_screen(parser.screen());
    assert_eq!(
        parser.screen().contents_formatted(),
        &b"\x1b[?25h\x1b[m\x1b[H\x1b[Jfoo\x1b[33;1;7mb\x1b[42;22ma\x1b[35mr\r\n\x1b[45mquux"[..]
    );

    parser
        .process(b"\x1b[2;2H\x1b[38;2;123;213;231mu\x1b[38;5;254mu\x1b[39mx");
    helpers::contents_formatted_reproduces_screen(parser.screen());
    assert_eq!(parser.screen().contents_formatted(), &b"\x1b[?25h\x1b[m\x1b[H\x1b[Jfoo\x1b[33;1;7mb\x1b[42;22ma\x1b[35mr\r\n\x1b[45mq\x1b[38;2;123;213;231mu\x1b[38;5;254mu\x1b[39mx"[..]);
}

#[test]
fn empty_cells() {
    let mut parser = vt100::Parser::default();
    parser.process(b"\x1b[5C\x1b[32m bar\x1b[H\x1b[31mfoo");
    helpers::contents_formatted_reproduces_screen(parser.screen());
    assert_eq!(parser.screen().contents(), "foo   bar");
    assert_eq!(
        parser.screen().contents_formatted(),
        &b"\x1b[?25h\x1b[m\x1b[H\x1b[J\x1b[31mfoo\x1b[2C\x1b[32m bar\x1b[1;4H\x1b[31m"[..]
    );
}

#[test]
fn cursor_positioning() {
    let mut parser = vt100::Parser::default();

    let screen = parser.screen().clone();
    parser.process(b":\x1b[K");
    assert_eq!(parser.screen().cursor_position(), (0, 1));
    assert_eq!(
        parser.screen().contents_formatted(),
        b"\x1b[?25h\x1b[m\x1b[H\x1b[J:"
    );
    assert_eq!(parser.screen().contents_diff(&screen), b":");

    let screen = parser.screen().clone();
    parser.process(b"a");
    assert_eq!(parser.screen().cursor_position(), (0, 2));
    assert_eq!(
        parser.screen().contents_formatted(),
        b"\x1b[?25h\x1b[m\x1b[H\x1b[J:a"
    );
    assert_eq!(parser.screen().contents_diff(&screen), b"a");

    let screen = parser.screen().clone();
    parser.process(b"\x1b[1;2H\x1b[K");
    assert_eq!(parser.screen().cursor_position(), (0, 1));
    assert_eq!(
        parser.screen().contents_formatted(),
        b"\x1b[?25h\x1b[m\x1b[H\x1b[J:"
    );
    assert_eq!(parser.screen().contents_diff(&screen), b"\x1b[1;2H\x1b[K");

    let screen = parser.screen().clone();
    parser.process(b"\x1b[H\x1b[J\x1b[4;80H");
    assert_eq!(parser.screen().cursor_position(), (3, 79));
    assert_eq!(
        parser.screen().contents_formatted(),
        b"\x1b[?25h\x1b[m\x1b[H\x1b[J\x1b[4;80H"
    );
    assert_eq!(
        parser.screen().contents_diff(&screen),
        b"\x1b[H\x1b[K\x1b[4;80H"
    );

    let screen = parser.screen().clone();
    parser.process(b"a");
    assert_eq!(parser.screen().cursor_position(), (3, 80));
    assert_eq!(
        parser.screen().contents_formatted(),
        b"\x1b[?25h\x1b[m\x1b[H\x1b[J\x1b[4;80Ha"
    );
    assert_eq!(parser.screen().contents_diff(&screen), b"a");

    let screen = parser.screen().clone();
    parser.process(b"\n");
    assert_eq!(parser.screen().cursor_position(), (4, 80));
    assert_eq!(
        parser.screen().contents_formatted(),
        b"\x1b[?25h\x1b[m\x1b[H\x1b[J\x1b[4;80Ha\n"
    );
    assert_eq!(parser.screen().contents_diff(&screen), b"\n");

    let screen = parser.screen().clone();
    parser.process(b"b");
    assert_eq!(parser.screen().cursor_position(), (5, 1));
    assert_eq!(
        parser.screen().contents_formatted(),
        b"\x1b[?25h\x1b[m\x1b[H\x1b[J\x1b[4;80Ha\x1b[6;1Hb"
    );
    assert_eq!(parser.screen().contents_diff(&screen), b"\r\nb");
}

#[test]
fn rows() {
    let mut parser = vt100::Parser::default();
    let screen1 = parser.screen().clone();
    assert_eq!(
        screen1.rows(0, 80).collect::<Vec<String>>(),
        vec![
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
        ]
    );
    assert_eq!(screen1.rows_formatted(0, 80).collect::<Vec<Vec<u8>>>(), {
        let x: Vec<Vec<u8>> = vec![
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
        ];
        x
    });
    assert_eq!(
        screen1.rows(5, 15).collect::<Vec<String>>(),
        vec![
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
        ]
    );
    assert_eq!(screen1.rows_formatted(5, 15).collect::<Vec<Vec<u8>>>(), {
        let x: Vec<Vec<u8>> = vec![
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
        ];
        x
    });

    parser
        .process(b"\x1b[31mfoo\x1b[10;10H\x1b[32mbar\x1b[20;20H\x1b[33mbaz");
    let screen2 = parser.screen().clone();
    assert_eq!(
        screen2.rows(0, 80).collect::<Vec<String>>(),
        vec![
            "foo".to_string(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            "         bar".to_string(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            "                   baz".to_string(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
        ]
    );
    assert_eq!(
        screen2.rows_formatted(0, 80).collect::<Vec<Vec<u8>>>(),
        vec![
            b"\x1b[31mfoo".to_vec(),
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            b"\x1b[9C\x1b[32mbar".to_vec(),
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            b"\x1b[19C\x1b[33mbaz".to_vec(),
            vec![],
            vec![],
            vec![],
            vec![],
        ]
    );
    assert_eq!(
        screen2.rows(5, 15).collect::<Vec<String>>(),
        vec![
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            "    bar".to_string(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            "              b".to_string(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
        ]
    );
    assert_eq!(
        screen2.rows_formatted(5, 15).collect::<Vec<Vec<u8>>>(),
        vec![
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            b"\x1b[4C\x1b[32mbar".to_vec(),
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            b"\x1b[14C\x1b[33mb".to_vec(),
            vec![],
            vec![],
            vec![],
            vec![],
        ]
    );

    assert_eq!(
        screen2.rows_diff(&screen1, 0, 80).collect::<Vec<Vec<u8>>>(),
        vec![
            b"\x1b[31mfoo".to_vec(),
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            b"\x1b[9C\x1b[32mbar".to_vec(),
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            b"\x1b[19C\x1b[33mbaz".to_vec(),
            vec![],
            vec![],
            vec![],
            vec![],
        ]
    );

    parser.process(b"\x1b[10;11Ho");
    let screen3 = parser.screen().clone();
    assert_eq!(
        screen3.rows_diff(&screen2, 0, 80).collect::<Vec<Vec<u8>>>(),
        vec![
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            b"\x1b[10C\x1b[33mo".to_vec(),
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
        ]
    );
}

#[test]
fn contents_between() {
    let mut parser = vt100::Parser::default();
    assert_eq!(parser.screen().contents_between(0, 0, 0, 0), "");
    assert_eq!(parser.screen().contents_between(0, 0, 5, 0), "\n\n\n\n\n");
    assert_eq!(parser.screen().contents_between(5, 0, 0, 0), "");

    parser.process(
        b"Lorem ipsum dolor sit amet, consectetur adipiscing elit, \
        sed do eiusmod tempor incididunt ut labore et dolore magna aliqua.\n\n\
        Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris \
        nisi ut aliquip ex ea commodo consequat.\n\n\
        Duis aute irure dolor in reprehenderit in voluptate velit esse cillum \
        dolore eu fugiat nulla pariatur.\n\n\
        Excepteur sint occaecat cupidatat non proident, sunt in culpa qui \
        officia deserunt mollit anim id est laborum.",
    );
    assert_eq!(parser.screen().contents_between(0, 0, 0, 0), "");
    assert_eq!(
        parser.screen().contents_between(0, 0, 0, 26),
        "Lorem ipsum dolor sit amet"
    );
    assert_eq!(parser.screen().contents_between(0, 26, 0, 0), "");
    assert_eq!(
        parser.screen().contents_between(0, 57, 1, 43),
        "sed do eiusmod tempor incididunt ut labore et dolore magna aliqua."
    );
    assert_eq!(
        parser.screen().contents_between(0, 57, 2, 0),
        "sed do eiusmod tempor incididunt ut labore et dolore magna aliqua.\n"
    );
    assert_eq!(parser.screen().contents_between(2, 0, 0, 57), "");
}

#[test]
fn diff_basic() {
    let mut parser = vt100::Parser::default();
    let screen1 = parser.screen().clone();
    parser.process(b"\x1b[5C\x1b[32m bar");
    let screen2 = parser.screen().clone();
    assert_eq!(screen2.contents_diff(&screen1), b"\x1b[5C\x1b[32m bar");
    helpers::assert_contents_diff_reproduces_state_from_screens(
        &screen1, &screen2,
    );

    parser.process(b"\x1b[H\x1b[31mfoo");
    let screen3 = parser.screen().clone();
    assert_eq!(screen3.contents_diff(&screen2), b"\x1b[H\x1b[31mfoo");
    helpers::assert_contents_diff_reproduces_state_from_screens(
        &screen2, &screen3,
    );

    parser.process(b"\x1b[1;7H\x1b[32mbaz");
    let screen4 = parser.screen().clone();
    assert_eq!(screen4.contents_diff(&screen3), b"\x1b[5C\x1b[32mz");
    helpers::assert_contents_diff_reproduces_state_from_screens(
        &screen3, &screen4,
    );

    parser.process(b"\x1b[1;8H\x1b[X");
    let screen5 = parser.screen().clone();
    assert_eq!(screen5.contents_diff(&screen4), b"\x1b[1;8H\x1b[X");
    helpers::assert_contents_diff_reproduces_state_from_screens(
        &screen4, &screen5,
    );
}

#[test]
fn diff_erase() {
    let mut parser = vt100::Parser::default();

    let screen = parser.screen().clone();
    parser.process(b"foo\x1b[5;5Hbar");
    assert_eq!(parser.screen().contents_diff(&screen), b"foo\x1b[5;5Hbar");

    let screen = parser.screen().clone();
    parser.process(b"\x1b[3D\x1b[2X");
    assert_eq!(parser.screen().contents_diff(&screen), b"\x1b[5;5H\x1b[2X");

    let screen = parser.screen().clone();
    parser.process(b"\x1bcfoo\x1b[5;5Hbar");
    assert_eq!(parser.screen().contents_diff(&screen), b"ba\x1b[C");

    let screen = parser.screen().clone();
    parser.process(b"\x1b[3D\x1b[3X");
    assert_eq!(parser.screen().contents_diff(&screen), b"\x1b[5;5H\x1b[K");
}
