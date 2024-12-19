# Changelog

## [Unreleased]

### Added

- `Parser::process_cb`, which works the same as `Parser::process` except that
  it calls callbacks during parsing when it finds a terminal escape which is
  potentially useful but not something that affects the screen itself.
- Support for xterm window resize request escape codes, via the new callback
  mechanism.

### Removed

- `Screen::bells_diff`, `Screen::audible_bell_count`,
  `Screen::visual_bell_count`, and `Screen::errors` have been removed in favor
  of the new callback api described above.
- `Cell` no longer implements `Default`.
- `Screen` no longer implements `vte::Perform`.

### Changed

- `Parser::set_size` and `Parser::set_scrollback` have been moved to methods
  on `Screen`, and `Parser::screen_mut` was added to get a mutable reference
  to the screen.

## [0.15.2] - 2023-02-05

### Changed

- Bumped dependencies

## [0.15.1] - 2021-12-21

### Changed

- Removed a lot of unnecessary test data from the packaged crate, making
  downloads faster

## [0.15.0] - 2021-12-15

### Added

- `Screen::errors` to track the number of parsing errors seen so far

### Fixed

- No longer generate spurious diffs in some cases where the cursor is past the
  end of a row
- Fix restoring the cursor position when scrolled back

### Changed

- Various internal refactorings

## [0.14.0] - 2021-12-06

### Changed

- Unknown UTF-8 characters default to a width of 1, rather than 0 (except for
  control characters, as mentioned below)

### Fixed

- Ignore C1 control characters rather than adding them to the cell data, since
  they are non-printable

## [0.13.2] - 2021-12-05

### Changed

- Delay allocation of the alternate screen until it is used (saves a bit of
  memory in basic cases)

## [0.13.1] - 2021-12-04

### Fixed

- Fixed various line wrapping state issues
- Fixed cursor positioning after writing zero width characters at the end of
  the line
- Fixed `Screen::cursor_state_formatted` to draw the last character in a line
  with the appropriate drawing attributes if it needs to redraw it

## [0.13.0] - 2021-11-17

### Added

- `Screen::alternate_screen` to determine if the alternate screen is in use
- `Screen::row_wrapped` to determine whether the row at the given index should
  wrap its text
- `Screen::cursor_state_formatted` to set the cursor position and hidden state
  (including internal state like the one-past-the-end state which isn't visible
  in the return value of `cursor_position`)

### Fixed

- `Screen::rows_formatted` now outputs correct escape codes in some edge cases
  at the beginning of a row when the previous row was wrapped
- VPA escape sequence can no longer position the cursor off the screen

## [0.12.0] - 2021-03-09

### Added

- `Screen::state_formatted` and `Screen::state_diff` convenience wrappers

### Fixed

- `Screen::attributes_formatted` now correctly resets previously set attributes
  where necessary

### Removed

- Removed `Screen::attributes_diff`, since I can't actually think of any
  situation where it does a thing that makes sense.

## [0.11.1] - 2021-03-07

### Changed

- Drop dependency on `enumset`

## [0.11.0] - 2021-03-07

### Added

- `Screen::attributes_formatted` and `Screen::attributes_diff` to retrieve the
  current state of the drawing attributes as escape sequences
- `Screen::fgcolor`, `Screen::bgcolor`, `Screen::bold`, `Screen::italic`,
  `Screen::underline`, and `Screen::inverse` to retrieve the current state of
  the drawing attributes directly

## [0.10.0] - 2021-03-06

### Added

- Implementation of `std::io::Write` for `Parser`

## [0.9.0] - 2021-03-05

### Added

- `Screen::contents_between`, for returning the contents logically between two
  given cells (for things like clipboard selection)
- Support SGR subparameters (so `\e[38:2:255:0:0m` behaves the same way as
  `\e[38;2;255;0;0m`)

### Fixed

- Bump `enumset` to fix a dependency which fails to build

## [0.8.1] - 2020-02-09

### Changed

- Bumped `vte` dep to 0.6.

## [0.8.0] - 2019-12-07

### Removed

- Removed the unicode-normalization feature altogether - it turns out that it
  still has a couple edge cases where it causes incorrect behavior, and fixing
  those would be a lot more effort.

### Fixed

- Fix a couple more end-of-line/wrapping bugs, especially around cursor
  positioning.
- Fix applying combining characters to wide characters.
- Ensure cells can't have contents with width zero (to avoid ambiguity). If an
  empty cell gets a combining character applied to it, default that cell to a
  (normal-width) space first.

## [0.7.0] - 2019-11-23

### Added

- New (default-on) cargo feature `unicode-normalization` which can be disabled
  to disable normalizing cell contents to NFC - it's a pretty small edge case,
  and the data tables required to support it are quite large, which affects
  size-sensitive targets like wasm

## [0.6.3] - 2019-11-20

### Fixed

- Fix output of `contents_formatted` and `contents_diff` when the cursor
  position ends at one past the end of a row.
- If the cursor position is one past the end of a row, any char, even a
  combining char, needs to cause the cursor position to wrap.

## [0.6.2] - 2019-11-13

### Fixed

- Fix zero-width characters when the cursor is at the end of a row.

## [0.6.1] - 2019-11-13

### Added

- Add more debug logging for unhandled escape sequences.

### Changed

- Unhandled escape sequence warnings are now at the `debug` log level.

## [0.6.0] - 2019-11-13

### Added

- `Screen::input_mode_formatted` and `Screen::input_mode_diff` give escape
  codes to set the current terminal input modes.
- `Screen::title_formatted` and `Screen::title_diff` give escape codes to set
  the terminal window title.
- `Screen::bells_diff` gives escape codes to trigger any audible or visual
  bells which have been seen since the previous state.

### Changed

- `Screen::contents_diff` no longer includes audible or visual bells (see
  `Screen::bells_diff` instead).

## [0.5.1] - 2019-11-12

### Fixed

- `Screen::set_size` now actually resizes when requested (previously the
  underlying storage was not being resized, leading to panics when writing
  outside of the original screen).

## [0.5.0] - 2019-11-12

### Added

- Scrollback support.
- `Default` impl for `Parser` which creates an 80x24 terminal with no
  scrollback.

### Removed

- `Parser::screen_mut` (and the `pub` `&mut self` methods on `Screen`). The few
  things you can do to change the screen state directly are now exposed as
  methods on `Parser` itself.

### Changed

- `Cell::contents` now returns a `String` instead of a `&str`.
- `Screen::check_audible_bell` and `Screen::check_visual_bell` have been
  replaced with `Screen::audible_bell_count` and `Screen::visual_bell_count`.
  You should keep track of the "since the last method call" state yourself
  instead of having the screen track it for you.

### Fixed

- Lots of performance and output optimizations.
- Clearing a cell now sets all of that cell's attributes to the current
  attribute set, since different terminals render different things for an empty
  cell based on the attributes.
- `Screen::contents_diff` now includes audible and visual bells when
  appropriate.

## [0.4.0] - 2019-11-08

### Removed

- `Screen::fgcolor`, `Screen::bgcolor`, `Screen::bold`, `Screen::italic`,
  `Screen::underline`, `Screen::inverse`, and `Screen::alternate_screen`:
  these are just implementation details that people shouldn't need to care
  about.

### Fixed

- Fixed cursor movement when the cursor position is already outside of an
  active scroll region.

## [0.3.2] - 2019-11-08

### Fixed

- Clearing cells now correctly sets the cell background color.
- Fixed a couple bugs in wide character handling in `contents_formatted` and
  `contents_diff`.
- Fixed RI when the cursor is at the top of the screen (fixes scrolling up in
  `less`, for instance).
- Fixed VPA incorrectly being clamped to the scroll region.
- Stop treating soft hyphen specially (as far as i can tell, no other terminals
  do this, and i'm not sure why i thought it was necessary to begin with).
- `contents_formatted` now also resets attributes at the start, like
  `contents_diff` does.

## [0.3.1] - 2019-11-06

### Fixed

- Make `contents_formatted` explicitly show the cursor when necessary, in case
  the cursor was previously hidden.

## [0.3.0] - 2019-11-06

### Added

- `Screen::rows` which is like `Screen::contents` except that it returns the
  data by row instead of all at once, and also allows you to restrict the
  region returned to a subset of columns.
- `Screen::rows_formatted` which is like `Screen::rows`, but returns escape
  sequences sufficient to draw the requested subset of each row.
- `Screen::contents_diff` and `Screen::rows_diff` which return escape sequences
  sufficient to turn the visible state of one screen (or a subset of the screen
  in the case of `rows_diff`) into another.

### Changed

- The screen is now exposed separately from the parser, and is cloneable.
- `contents_formatted` now returns `Vec<u8>` instead of `String`.
- `contents` and `contents_formatted` now only allow getting the contents of
  the entire screen rather than a subset (but see the entry for `rows` and
  `rows_formatted` above).

### Removed

- `Cell::new`, since there's not really any reason that this is useful for
  someone to do from outside of the crate.

### Fixed

- `contents_formatted` now preserves the state of empty cells instead of
  filling them with spaces.
- We now clear the row wrapping state when the number of columns in the
  terminal is changed.
- `contents_formatted` now ensures that the cursor has the correct hidden state
  and location.
- `contents_formatted` now clears the screen before starting to draw.

## [0.2.0] - 2019-11-04

### Changed

- Reimplemented in pure safe rust, with a much more accurate parser
- A bunch of minor API tweaks, some backwards-incompatible

## [0.1.2] - 2016-06-04

### Fixed

- Fix returning uninit memory in get_string_formatted/get_string_plaintext
- Handle emoji and zero width unicode characters properly
- Fix cursor positioning with regards to scroll regions and wrapping
- Fix parsing of (ignored) character set escapes
- Explicitly suppress status report escapes

## [0.1.1] - 2016-04-28

### Fixed

- Fix builds

## [0.1.0] - 2016-04-28

### Added

- Initial release
