use std::fmt::{Display, Formatter, Result};

pub struct PrettyPrintError<'a>(pub &'a anyhow::Error);

impl<'a> Display for PrettyPrintError<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        let mut i = 0;
        let mut has_details = false;

        let descriptions = self
            .0
            .chain()
            .map(|cause| cause.to_string())
            .collect::<Vec<_>>();

        for description in &descriptions {
            let hidden = description.starts_with("Execution of ");
            if !hidden {
                let header =
                    description
                        .split_once('\n')
                        .map_or(description.as_str(), |(header, _)| {
                            has_details = true;
                            header
                        });
                match i {
                    0 => write!(f, "{}", header)?,
                    1 => write!(f, "\n\nCaused by:\n- {}", header)?,
                    _ => write!(f, "\n- {}", header)?,
                }
                i += 1;
            } else {
                has_details = true;
            }
        }
        if has_details {
            write!(f, "\n\nDeveloper details:")?;
            for description in descriptions {
                f.write_str("\n");
                WithDash(&description).fmt(f)?;
            }
        }
        Ok(())
    }
}

struct WithDash<'a>(&'a str);

impl<'a> Display for WithDash<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        let mut lines = self.0.lines();
        if let Some(line) = lines.next() {
            write!(f, "- {}", line)?;
        }
        for line in lines {
            write!(f, "\n  {}", line)?;
        }
        Ok(())
    }
}
