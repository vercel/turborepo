# turborepo-vt100

This is a fork of vt100 modified for use by turborepo.
Most changes made can be upstreamed and hopefully one day we won't need to vendor the crate.

## Changes

This fork is based on commit `1e4014aa72a7552d2f69b81ad89d56e035354041` of `vt100`.
A few immediate changes made when vendoring:

- Dropped fuzz tests as they require special setup
- Dropped scripts for helping with fuzz testing

## Original README of vt100

This crate parses a terminal byte stream and provides an in-memory
representation of the rendered contents.

### Overview

This is essentially the terminal parser component of a graphical terminal
emulator pulled out into a separate crate. Although you can use this crate
to build a graphical terminal emulator, it also contains functionality
necessary for implementing terminal applications that want to run other
terminal applications - programs like `screen` or `tmux` for example.

### Synopsis

```rust
let mut parser = vt100::Parser::new(24, 80, 0);

let screen = parser.screen().clone();
parser.process(b"this text is \x1b[31mRED\x1b[m");
assert_eq!(
    parser.screen().cell(0, 13).unwrap().fgcolor(),
    vt100::Color::Idx(1),
);

let screen = parser.screen().clone();
parser.process(b"\x1b[3D\x1b[32mGREEN");
assert_eq!(
    parser.screen().contents_formatted(),
    &b"\x1b[?25h\x1b[m\x1b[H\x1b[Jthis text is \x1b[32mGREEN"[..],
);
assert_eq!(
    parser.screen().contents_diff(&screen),
    &b"\x1b[1;14H\x1b[32mGREEN"[..],
);
```
