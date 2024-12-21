use turborepo_vt100 as vt100;

use serde::de::Deserialize as _;
use std::io::Read as _;

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct FixtureCell {
    contents: String,
    #[serde(default, skip_serializing_if = "is_default")]
    is_wide: bool,
    #[serde(default, skip_serializing_if = "is_default")]
    is_wide_continuation: bool,
    #[serde(
        default,
        deserialize_with = "deserialize_color",
        serialize_with = "serialize_color",
        skip_serializing_if = "is_default"
    )]
    fgcolor: vt100::Color,
    #[serde(
        default,
        deserialize_with = "deserialize_color",
        serialize_with = "serialize_color",
        skip_serializing_if = "is_default"
    )]
    bgcolor: vt100::Color,
    #[serde(default, skip_serializing_if = "is_default")]
    bold: bool,
    #[serde(default, skip_serializing_if = "is_default")]
    italic: bool,
    #[serde(default, skip_serializing_if = "is_default")]
    underline: bool,
    #[serde(default, skip_serializing_if = "is_default")]
    inverse: bool,
}

impl FixtureCell {
    #[allow(dead_code)]
    pub fn from_cell(cell: &vt100::Cell) -> Self {
        Self {
            contents: cell.contents().to_string(),
            is_wide: cell.is_wide(),
            is_wide_continuation: cell.is_wide_continuation(),
            fgcolor: cell.fgcolor(),
            bgcolor: cell.bgcolor(),
            bold: cell.bold(),
            italic: cell.italic(),
            underline: cell.underline(),
            inverse: cell.inverse(),
        }
    }
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct FixtureScreen {
    contents: String,
    cells: std::collections::BTreeMap<String, FixtureCell>,
    cursor_position: (u16, u16),
    #[serde(default, skip_serializing_if = "is_default")]
    title: String,
    #[serde(default, skip_serializing_if = "is_default")]
    icon_name: String,
    #[serde(default, skip_serializing_if = "is_default")]
    application_keypad: bool,
    #[serde(default, skip_serializing_if = "is_default")]
    application_cursor: bool,
    #[serde(default, skip_serializing_if = "is_default")]
    hide_cursor: bool,
    #[serde(default, skip_serializing_if = "is_default")]
    bracketed_paste: bool,
    #[serde(
        default,
        deserialize_with = "deserialize_mouse_protocol_mode",
        serialize_with = "serialize_mouse_protocol_mode",
        skip_serializing_if = "is_default"
    )]
    mouse_protocol_mode: vt100::MouseProtocolMode,
    #[serde(
        default,
        deserialize_with = "deserialize_mouse_protocol_encoding",
        serialize_with = "serialize_mouse_protocol_encoding",
        skip_serializing_if = "is_default"
    )]
    mouse_protocol_encoding: vt100::MouseProtocolEncoding,
}

impl FixtureScreen {
    fn load<R: std::io::Read>(r: R) -> Self {
        serde_json::from_reader(r).unwrap()
    }

    #[allow(dead_code)]
    pub fn from_screen(screen: &vt100::Screen) -> Self {
        let empty_screen = vt100::Parser::default().screen().clone();
        let empty_cell = empty_screen.cell(0, 0).unwrap();
        let mut cells = std::collections::BTreeMap::new();
        let (rows, cols) = screen.size();
        for row in 0..rows {
            for col in 0..cols {
                let cell = screen.cell(row, col).unwrap();
                if cell != empty_cell {
                    cells.insert(
                        format!("{row},{col}"),
                        FixtureCell::from_cell(cell),
                    );
                }
            }
        }
        Self {
            contents: screen.contents(),
            cells,
            cursor_position: screen.cursor_position(),
            title: screen.title().to_string(),
            icon_name: screen.icon_name().to_string(),
            application_keypad: screen.application_keypad(),
            application_cursor: screen.application_cursor(),
            hide_cursor: screen.hide_cursor(),
            bracketed_paste: screen.bracketed_paste(),
            mouse_protocol_mode: screen.mouse_protocol_mode(),
            mouse_protocol_encoding: screen.mouse_protocol_encoding(),
        }
    }
}

fn is_default<T: Default + PartialEq>(t: &T) -> bool {
    t == &T::default()
}

fn deserialize_color<'a, D>(
    deserializer: D,
) -> std::result::Result<vt100::Color, D::Error>
where
    D: serde::de::Deserializer<'a>,
{
    let val = <Option<String>>::deserialize(deserializer)?;
    match val {
        None => Ok(vt100::Color::Default),
        Some(x) if x.starts_with('#') => {
            let x = x.as_bytes();
            if x.len() != 7 {
                return Err(serde::de::Error::custom("invalid rgb color"));
            }
            let r =
                super::hex(x[1], x[2]).map_err(serde::de::Error::custom)?;
            let g =
                super::hex(x[3], x[4]).map_err(serde::de::Error::custom)?;
            let b =
                super::hex(x[5], x[6]).map_err(serde::de::Error::custom)?;
            Ok(vt100::Color::Rgb(r, g, b))
        }
        Some(x) => Ok(vt100::Color::Idx(
            x.parse().map_err(serde::de::Error::custom)?,
        )),
    }
}

fn serialize_color<S>(
    color: &vt100::Color,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let s = match color {
        vt100::Color::Default => unreachable!(),
        vt100::Color::Idx(n) => format!("{n}"),
        vt100::Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
    };
    serializer.serialize_str(&s)
}

fn deserialize_mouse_protocol_mode<'a, D>(
    deserializer: D,
) -> std::result::Result<vt100::MouseProtocolMode, D::Error>
where
    D: serde::de::Deserializer<'a>,
{
    let name = <String>::deserialize(deserializer)?;
    match name.as_ref() {
        "none" => Ok(vt100::MouseProtocolMode::None),
        "press" => Ok(vt100::MouseProtocolMode::Press),
        "press_release" => Ok(vt100::MouseProtocolMode::PressRelease),
        "button_motion" => Ok(vt100::MouseProtocolMode::ButtonMotion),
        "any_motion" => Ok(vt100::MouseProtocolMode::AnyMotion),
        _ => unimplemented!(),
    }
}

fn serialize_mouse_protocol_mode<S>(
    mode: &vt100::MouseProtocolMode,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let s = match mode {
        vt100::MouseProtocolMode::None => "none",
        vt100::MouseProtocolMode::Press => "press",
        vt100::MouseProtocolMode::PressRelease => "press_release",
        vt100::MouseProtocolMode::ButtonMotion => "button_motion",
        vt100::MouseProtocolMode::AnyMotion => "any_motion",
    };
    serializer.serialize_str(s)
}

fn deserialize_mouse_protocol_encoding<'a, D>(
    deserializer: D,
) -> std::result::Result<vt100::MouseProtocolEncoding, D::Error>
where
    D: serde::de::Deserializer<'a>,
{
    let name = <String>::deserialize(deserializer)?;
    match name.as_ref() {
        "default" => Ok(vt100::MouseProtocolEncoding::Default),
        "utf8" => Ok(vt100::MouseProtocolEncoding::Utf8),
        "sgr" => Ok(vt100::MouseProtocolEncoding::Sgr),
        _ => unimplemented!(),
    }
}

fn serialize_mouse_protocol_encoding<S>(
    encoding: &vt100::MouseProtocolEncoding,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let s = match encoding {
        vt100::MouseProtocolEncoding::Default => "default",
        vt100::MouseProtocolEncoding::Utf8 => "utf8",
        vt100::MouseProtocolEncoding::Sgr => "sgr",
    };
    serializer.serialize_str(s)
}

fn load_input(name: &str, i: usize) -> Option<Vec<u8>> {
    let mut file = std::fs::File::open(format!(
        "tests/data/fixtures/{name}/{i}.typescript"
    ))
    .ok()?;
    let mut input = vec![];
    file.read_to_end(&mut input).unwrap();
    Some(input)
}

fn load_screen(name: &str, i: usize) -> Option<FixtureScreen> {
    let mut file =
        std::fs::File::open(format!("tests/data/fixtures/{name}/{i}.json"))
            .ok()?;
    Some(FixtureScreen::load(&mut file))
}

fn assert_produces(input: &[u8], expected: &FixtureScreen) {
    let mut parser = vt100::Parser::default();
    parser.process(input);

    assert_eq!(parser.screen().contents(), expected.contents);
    assert_eq!(parser.screen().cursor_position(), expected.cursor_position);
    assert_eq!(parser.screen().title(), expected.title);
    assert_eq!(parser.screen().icon_name(), expected.icon_name);
    assert_eq!(
        parser.screen().application_keypad(),
        expected.application_keypad
    );
    assert_eq!(
        parser.screen().application_cursor(),
        expected.application_cursor
    );
    assert_eq!(parser.screen().hide_cursor(), expected.hide_cursor);
    assert_eq!(parser.screen().bracketed_paste(), expected.bracketed_paste);
    assert_eq!(
        parser.screen().mouse_protocol_mode(),
        expected.mouse_protocol_mode
    );
    assert_eq!(
        parser.screen().mouse_protocol_encoding(),
        expected.mouse_protocol_encoding
    );

    let (rows, cols) = parser.screen().size();
    for row in 0..rows {
        for col in 0..cols {
            let expected_cell = expected
                .cells
                .get(&format!("{row},{col}"))
                .cloned()
                .unwrap_or_default();
            let got_cell = parser.screen().cell(row, col).unwrap();
            assert_eq!(got_cell.contents(), expected_cell.contents);
            assert_eq!(got_cell.is_wide(), expected_cell.is_wide);
            assert_eq!(
                got_cell.is_wide_continuation(),
                expected_cell.is_wide_continuation
            );
            assert_eq!(got_cell.fgcolor(), expected_cell.fgcolor);
            assert_eq!(got_cell.bgcolor(), expected_cell.bgcolor);
            assert_eq!(got_cell.bold(), expected_cell.bold);
            assert_eq!(got_cell.italic(), expected_cell.italic);
            assert_eq!(got_cell.underline(), expected_cell.underline);
            assert_eq!(got_cell.inverse(), expected_cell.inverse);
        }
    }
}

#[allow(dead_code)]
pub fn fixture(name: &str) {
    let mut i = 1;
    let mut prev_input = vec![];
    while let Some(input) = load_input(name, i) {
        super::assert_reproduces_state_from(&input, &prev_input);
        prev_input.extend(input);

        let expected = load_screen(name, i).unwrap();
        assert_produces(&prev_input, &expected);

        i += 1;
    }
    assert!(i > 1, "couldn't find fixtures to test");
}
