use crate::ui::colors::ColorMode;
use crate::Args;

mod colors;

fn get_ui(args: &Args) {
    let mut color_mode = ColorMode::get_from_env();
    if args.no_color {
        color_mode = ColorMode::Suppressed;
    }
    if args.color {
        color_mode = ColorMode::Forced;
    }
}

fn build_colored_ui(color_mode: ColorMode) {}

fn apply_color_mode(color_mode: ColorMode) {}
