use colored::*;
use terminal_size::{terminal_size, Width};

mod utils;

const PADDING: usize = 8;
const TOP_LEFT: &str = "╭";
const TOP_RIGHT: &str = "╮";
const BOTTOM_LEFT: &str = "╰";
const BOTTOM_RIGHT: &str = "╯";
const HORIZONTAL: &str = "─";
const VERTICAL: &str = "│";
const SPACE: &str = " ";

enum BorderAlignment {
    Divider,
    Top,
    Bottom,
}

fn x_border(width: usize, position: BorderAlignment) {
    match position {
        BorderAlignment::Top => {
            println!(
                "{}{}{}",
                TOP_LEFT.yellow(),
                HORIZONTAL.repeat(width).yellow(),
                TOP_RIGHT.yellow()
            );
        }
        BorderAlignment::Bottom => {
            println!(
                "{}{}{}",
                BOTTOM_LEFT.yellow(),
                HORIZONTAL.repeat(width).yellow(),
                BOTTOM_RIGHT.yellow()
            );
        }
        BorderAlignment::Divider => {
            println!("{}", HORIZONTAL.repeat(width).yellow(),);
        }
    }
}

pub fn rectangle(text: &str) {
    let size = terminal_size();
    let lines: Vec<&str> = text.split("\n").map(|line| line.trim()).collect();
    // get the display width of each line so we can center it within the box later
    let lines_display_width: Vec<usize> = lines
        .iter()
        .map(|line| utils::get_display_length(line).unwrap())
        .collect();
    let longest_line = lines_display_width.iter().max().unwrap().to_owned();
    let full_message_width = longest_line + PADDING;

    // handle smaller viewports
    if let Some((Width(term_width), _)) = size {
        // we can't fit the box, so don't show it
        let term_width = usize::from(term_width) - 2;
        let cant_fit_box = full_message_width >= term_width;

        // can't fit the box or center every line, so left align
        if cant_fit_box && longest_line > term_width {
            // top border
            x_border(term_width, BorderAlignment::Divider);
            for (idx, line) in lines.iter().enumerate() {
                let line_display_width = lines_display_width[idx];
                if line_display_width == 0 {
                    println!("{}", SPACE.repeat(term_width));
                } else {
                    println!("{}", line);
                }
            }
            // bottom border
            x_border(term_width, BorderAlignment::Divider);
            return;
        }

        // can't fit the box, but we can still center text
        if cant_fit_box {
            // top border
            x_border(term_width, BorderAlignment::Divider);
            for (idx, line) in lines.iter().enumerate() {
                let line_display_width = lines_display_width[idx];
                if line_display_width == 0 {
                    println!("{}", SPACE.repeat(term_width));
                } else {
                    let line_padding = (term_width - lines_display_width[idx]) / 2;
                    // for lines of odd length, tack the reminder to the end
                    let line_padding_remainder =
                        term_width - (line_padding * 2) - lines_display_width[idx];
                    println!(
                        "{}{}{}",
                        SPACE.repeat(line_padding),
                        line,
                        SPACE.repeat(line_padding + line_padding_remainder),
                    );
                }
            }
            // bottom border
            x_border(term_width, BorderAlignment::Divider);
            return;
        }
    }

    // full output
    x_border(full_message_width, BorderAlignment::Top);
    for (idx, line) in lines.iter().enumerate() {
        if line.len() == 0 {
            println!(
                "{}{}{}",
                VERTICAL.yellow(),
                SPACE.repeat(full_message_width),
                VERTICAL.yellow()
            );
        } else {
            let line_padding = (full_message_width - lines_display_width[idx]) / 2;
            // for lines of odd length, tack the reminder to the end
            let line_padding_remainder =
                full_message_width - (line_padding * 2) - lines_display_width[idx];
            println!(
                "{}{}{}{}{}",
                VERTICAL.yellow(),
                SPACE.repeat(line_padding),
                line,
                SPACE.repeat(line_padding + line_padding_remainder),
                VERTICAL.yellow()
            );
        }
    }
    x_border(full_message_width, BorderAlignment::Bottom);
}
