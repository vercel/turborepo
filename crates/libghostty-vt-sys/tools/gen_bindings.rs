use std::{env, path::PathBuf};

use bindgen::{
    EnumVariation,
    callbacks::{EnumVariantValue, IntKind, ItemInfo, ItemKind, ParseCallbacks},
};
use heck::ToShoutySnakeCase;

fn main() {
    // The headers this crate's build script installed for this very build,
    // so they always match the pin and the enabled features. GHOSTTY_SOURCE_DIR
    // or an explicit GHOSTTY_INCLUDE_DIR override them.
    let include_dir = if let Ok(dir) = env::var("GHOSTTY_INCLUDE_DIR") {
        PathBuf::from(dir)
    } else if let Ok(src) = env::var("GHOSTTY_SOURCE_DIR") {
        PathBuf::from(src).join("include")
    } else {
        let dir = PathBuf::from(option_env!("LIBGHOSTTY_VT_SYS_INCLUDE_DIR").unwrap_or_else(
            || {
                panic!(
                    "the build script installed no ghostty headers (pkg-config build?); set \
                     GHOSTTY_INCLUDE_DIR or GHOSTTY_SOURCE_DIR"
                )
            },
        ));
        assert!(
            dir.join("ghostty").join("vt.h").exists(),
            "no ghostty headers in {}; set GHOSTTY_INCLUDE_DIR or GHOSTTY_SOURCE_DIR",
            dir.display()
        );
        dir
    };

    let header = include_dir.join("ghostty").join("vt.h");
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set"));
    let out = manifest_dir.join("src").join("bindings.rs");

    let mut builder = bindgen::Builder::default()
        .header(header.to_string_lossy())
        .clang_arg(format!("-I{}", include_dir.to_string_lossy()))
        // Ghostty's ABI and the Rust wrapper use signed c_int enums. C++11
        // selects GHOSTTY_ENUM_TYPED's explicit `: int` even with libclang
        // versions that lack the c_fixed_enum extension in C mode.
        .clang_args(["-x", "c++", "-std=c++11"])
        .allowlist_function("[Gg]hostty.*")
        .allowlist_type("[Gg]hostty.*")
        .allowlist_var("GHOSTTY_.*")
        // Only used to force enums to `int` size. It is defined as `INT_MAX`,
        // and whether bindgen can evaluate that depends on which `limits.h`
        // clang resolves, which differs between environments. Exclude it so
        // the output is the same everywhere.
        .blocklist_item("GHOSTTY_ENUM_MAX_VALUE")
        .generate_cstr(true)
        .derive_default(true)
        .size_t_is_usize(true)
        .default_enum_style(EnumVariation::ModuleConsts)
        .parse_callbacks(Box::new(Callbacks));

    if cfg!(target_os = "linux") {
        builder = builder.clang_arg("-I/usr/include");
    }

    let bindings = builder
        .generate()
        .expect("failed to generate bindings from include/ghostty/vt.h");

    bindings
        .write_to_file(&out)
        .unwrap_or_else(|error| panic!("failed to write bindings to {}: {error}", out.display()));
}

const PREFIXES: &[(&str, &str)] = &[
    ("GhosttySearchOption", "GHOSTTY_SEARCH_OPT"),
    ("GhosttySysOption", "GHOSTTY_SYS_OPT"),
    (
        "GhosttyTerminalUnknownSequenceTag",
        "GHOSTTY_TERMINAL_UNKNOWN_SEQUENCE",
    ),
    ("GhosttyOptimizeMode", "GHOSTTY_OPTIMIZE"),
    ("GhosttyKeyEncoderOption", "GHOSTTY_KEY_ENCODER_OPT"),
    ("GhosttyMouseTrackingMode", "GHOSTTY_MOUSE_TRACKING"),
    ("GhosttyMouseEncoderOption", "GHOSTTY_MOUSE_ENCODER_OPT"),
    ("GhosttySgrAttributeTag", "GHOSTTY_SGR_ATTR"),
    ("GhosttyOscCommandData", "GHOSTTY_OSC_DATA"),
    ("GhosttyOscCommandType", "GHOSTTY_OSC_COMMAND"),
    ("GhosttyOscOption", "GHOSTTY_OSC_OPT"),
    ("GhosttyTerminalOption", "GHOSTTY_TERMINAL_OPT"),
    (
        "GhosttyTerminalScrollViewportTag",
        "GHOSTTY_SCROLL_VIEWPORT",
    ),
    ("GhosttyStyleColorTag", "GHOSTTY_STYLE_COLOR"),
    ("GhosttyRowSemanticPrompt", "GHOSTTY_ROW_SEMANTIC"),
    ("GhosttyCellSemanticContent", "GHOSTTY_CELL_SEMANTIC"),
    ("GhosttyCellContentTag", "GHOSTTY_CELL_CONTENT"),
    ("GhosttySizeReportStyle", "GHOSTTY_SIZE_REPORT"),
    ("GhosttyModeReportState", "GHOSTTY_MODE_REPORT"),
    ("GhosttyFocusEvent", "GHOSTTY_FOCUS"),
    ("GhosttyResult", "GHOSTTY_"),
    ("GhosttyKittyGraphicsImageData", "GHOSTTY_KITTY_IMAGE_DATA"),
    (
        "GhosttySelectionGestureEventOption",
        "GHOSTTY_SELECTION_GESTURE_EVENT_OPT",
    ),
    (
        "GhosttySnapshotDecoderOption",
        "GHOSTTY_SNAPSHOT_DECODER_OPT",
    ),
];

#[derive(Debug)]
struct Callbacks;

impl ParseCallbacks for Callbacks {
    fn item_name(&self, item_info: ItemInfo) -> Option<String> {
        let prefix = match item_info.kind {
            // Do not rename functions since bindgen unconditionally prefixes
            // the `link_name` with `\u{1}`, which was supposed to stop LLVM
            // from mangling the name again but apparently this is necessary
            // on macOS and other Apple platforms?
            //
            // Honestly, what the hell. See:
            // https://github.com/rust-lang/rust-bindgen/issues/1221
            ItemKind::Function => return None,
            ItemKind::Var => "GHOSTTY_",
            _ => "Ghostty",
        };
        Some(item_info.name.trim_start_matches(prefix).to_string())
    }

    fn enum_variant_name(
        &self,
        enum_name: Option<&str>,
        original_variant_name: &str,
        _variant_value: EnumVariantValue,
    ) -> Option<String> {
        let enum_name = enum_name?;

        // Remove redundant C prefixes
        let prefix = PREFIXES
            .iter()
            .find(|(v, _)| *v == enum_name)
            .map(|(_, n)| n.to_string())
            .unwrap_or(enum_name.to_shouty_snake_case());

        let transformed = original_variant_name
            .trim_start_matches(&prefix)
            .trim_start_matches('_');

        Some(transformed.to_string())
    }

    fn process_comment(&self, comment: &str) -> Option<String> {
        Some(
            comment
                .lines()
                // Ignore doxygen directives.
                .filter(|s| !s.trim().starts_with("@"))
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }

    fn int_macro(&self, name: &str, _value: i64) -> Option<IntKind> {
        // Fixup some int macro types to reduce manual casting
        if name.starts_with("GHOSTTY_DA_") || name.starts_with("GHOSTTY_MODS_") {
            Some(IntKind::U16)
        } else if name.starts_with("GHOSTTY_KITTY_KEY_") || name.starts_with("GHOSTTY_COLOR_NAMED_")
        {
            Some(IntKind::U8)
        } else {
            None
        }
    }
}
