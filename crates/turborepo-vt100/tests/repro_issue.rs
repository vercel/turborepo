use turborepo_vt100 as vt100;

#[test]
fn col_wrap_with_invalid_row_does_not_panic() {
    // This test verifies that col_wrap handles the case where drawing_row_mut
    // returns None gracefully, without panicking.

    let mut parser = vt100::Parser::new(24, 80, 0);

    // Create a scenario where we have a very small terminal
    parser.screen_mut().set_size(2, 5);

    // Fill the screen to trigger scrolling
    parser.process(b"12345"); // Fill first row
    parser.process(b"67890"); // This should wrap and trigger col_wrap
    parser.process(b"abcde"); // This should cause more scrolling
    parser.process(b"fghij"); // And more scrolling

    // At this point, we should have scrolled enough that some row references
    // in col_wrap might be invalid. The key is that this should not panic.

    // Try to trigger more wrapping with a wide character that might cause
    // the edge case where prev_pos.row is out of bounds
    parser.process(b"klmno");
    parser.process(b"pqrst");

    // If we get here without panicking, the fix is working
    assert_eq!(parser.screen().size(), (2, 5));
}

#[test]
fn col_wrap_edge_case_with_scrolling() {
    // Another test to specifically target the edge case in col_wrap
    let mut parser = vt100::Parser::new(3, 10, 0);

    // Fill multiple lines to cause scrolling
    for i in 0..10 {
        let line = format!("line{:06}", i);
        parser.process(line.as_bytes());
        parser.process(b"\r\n");
    }

    // Now try to trigger col_wrap with a scenario that might cause
    // drawing_row_mut to return None
    parser.process(b"1234567890"); // Exactly fill the width
    parser.process(b"X"); // This should trigger col_wrap

    // The test passes if we don't panic
    assert!(parser.screen().contents().contains("X"));
}
