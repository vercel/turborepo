use std::io::Read as _;

#[path = "../tests/helpers/mod.rs"]
mod helpers;

fn check_full(vt_base: &vt100::Screen, idx: usize) {
    let mut input = vec![];
    input.extend(vt_base.state_formatted());
    let mut vt_full = vt100::Parser::default();
    vt_full.process(&input);
    assert!(
        helpers::compare_screens(vt_full.screen(), vt_base),
        "{}: full:\n{}",
        idx,
        helpers::format_bytes(&input),
    );
}

fn check_diff_empty(
    vt_base: &vt100::Screen,
    empty: &vt100::Screen,
    idx: usize,
) {
    let mut input = vec![];
    input.extend(vt_base.state_diff(empty));
    let mut vt_diff_empty = vt100::Parser::default();
    vt_diff_empty.process(&input);
    assert!(
        helpers::compare_screens(vt_diff_empty.screen(), vt_base),
        "{}: diff-empty:\n{}",
        idx,
        helpers::format_bytes(&input),
    );
}

fn check_diff(
    vt_base: &vt100::Screen,
    vt_diff: &mut vt100::Parser,
    prev: &vt100::Screen,
    idx: usize,
) {
    let mut input = vec![];
    input.extend(vt_base.state_diff(prev));
    vt_diff.process(&input);
    assert!(
        helpers::compare_screens(vt_diff.screen(), vt_base),
        "{}: diff:\n{}",
        idx,
        helpers::format_bytes(&input),
    );
}

fn check_rows(vt_base: &vt100::Screen, idx: usize) {
    let mut input = vec![];
    let mut wrapped = false;
    for (idx, row) in vt_base.rows_formatted(0, 80).enumerate() {
        input.extend(b"\x1b[m");
        if !wrapped {
            input.extend(format!("\x1b[{}H", idx + 1).as_bytes());
        }
        input.extend(&row);
        wrapped = vt_base.row_wrapped(idx.try_into().unwrap());
    }
    input.extend(b"\x1b[m");
    input.extend(&vt_base.cursor_state_formatted());
    input.extend(&vt_base.attributes_formatted());
    input.extend(&vt_base.input_mode_formatted());
    input.extend(&vt_base.title_formatted());
    let mut vt_rows = vt100::Parser::default();
    vt_rows.process(&input);
    assert!(
        helpers::compare_screens(vt_rows.screen(), vt_base),
        "{}: rows:\n{}",
        idx,
        helpers::format_bytes(&input),
    );
}

fn read_byte() -> Option<u8> {
    let mut byte = [0];
    match std::io::stdin().read(&mut byte) {
        Ok(bytes) => {
            if bytes != 1 {
                return None;
            }
        }
        Err(e) => {
            eprintln!("{e}");
            return None;
        }
    }
    Some(byte[0])
}

fn main() {
    let mut vt_base = vt100::Parser::default();
    let mut vt_diff = vt100::Parser::default();
    let mut prev_screen = vt_base.screen().clone();
    let empty_screen = vt100::Parser::default().screen().clone();
    let mut idx = 0;
    while let Some(byte) = read_byte() {
        vt_base.process(&[byte]);

        check_full(vt_base.screen(), idx);
        check_diff_empty(vt_base.screen(), &empty_screen, idx);
        check_diff(vt_base.screen(), &mut vt_diff, &prev_screen, idx);
        check_rows(vt_base.screen(), idx);

        prev_screen = vt_base.screen().clone();
        idx += 1;
    }
}
