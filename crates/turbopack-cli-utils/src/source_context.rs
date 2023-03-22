use std::fmt::{Display, Write};

use owo_colors::OwoColorize;
use turbo_tasks_fs::source_context::{SourceContextLine, SourceContextLines};

struct MarkerRange {
    start: char,
    end: char,
    pos: usize,
    len: usize,
}

impl Display for MarkerRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for _ in 0..self.pos {
            f.write_char(' ')?;
        }
        f.write_char(self.start)?;
        if self.len > 1 {
            for _ in 2..self.len {
                f.write_char('-')?;
            }
            f.write_char(self.end)?;
        }
        Ok(())
    }
}

pub fn format_source_context_lines(ctx: &SourceContextLines, f: &mut impl Write) {
    let padding = 6;
    let space = ' ';
    for line in &ctx.0 {
        match line {
            SourceContextLine::Context { line, outside } => {
                writeln!(
                    f,
                    "{}",
                    format_args!("{line:>padding$} | {outside}").dimmed()
                )
                .unwrap();
            }
            SourceContextLine::Start {
                line,
                before,
                inside,
            } => {
                writeln!(
                    f,
                    "{space:padding$} | {}",
                    MarkerRange {
                        start: 'v',
                        end: '-',
                        pos: before.len(),
                        len: inside.len(),
                    }
                    .bold(),
                )
                .unwrap();
                writeln!(f, "{line:>padding$} + {}{}", before.dimmed(), inside.bold()).unwrap();
            }
            SourceContextLine::End {
                line,
                inside,
                after,
            } => {
                writeln!(f, "{line:>padding$} + {}{}", inside.bold(), after.dimmed()).unwrap();
                writeln!(
                    f,
                    "{space:padding$} +{}",
                    MarkerRange {
                        start: '-',
                        end: '^',
                        pos: 0,
                        len: inside.len() + 1,
                    }
                    .bold()
                )
                .unwrap();
            }
            SourceContextLine::StartAndEnd {
                line,
                before,
                inside,
                after,
            } => {
                writeln!(
                    f,
                    "{space:padding$} + {}",
                    MarkerRange {
                        start: 'v',
                        end: 'v',
                        pos: before.len(),
                        len: inside.len(),
                    }
                    .bold()
                )
                .unwrap();
                if inside.len() >= 2 {
                    writeln!(
                        f,
                        "{line:>padding$} + {}{}{}",
                        before.dimmed(),
                        inside.bold(),
                        after.dimmed()
                    )
                    .unwrap();
                } else {
                    writeln!(
                        f,
                        "{line:>padding$} + {}{}{}",
                        before.bold(),
                        inside.bold(),
                        after.bold()
                    )
                    .unwrap();
                }
                writeln!(
                    f,
                    "{space:padding$} + {}",
                    MarkerRange {
                        start: '^',
                        end: '^',
                        pos: before.len(),
                        len: inside.len(),
                    }
                    .bold()
                )
                .unwrap();
            }
            SourceContextLine::Inside { line, inside } => {
                writeln!(f, "{:>padding$} + {}", line.bold(), inside.bold()).unwrap();
            }
        }
    }
}
