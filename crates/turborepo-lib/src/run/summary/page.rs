use maud::{html, Markup, DOCTYPE};
use tailwind_css::TailwindBuilder;

use crate::run::summary::{execution::ExecutionSummary, Error, RunSummary};

impl<'a> RunSummary<'a> {
    pub fn render_html(&self) -> Result<String, Error> {
        let mut tailwind = TailwindBuilder::default();
        let body = html! {
        body class=(trace(&mut tailwind, "flex flex-col")?) {
            h3 class=(trace(&mut tailwind, "text-md text-gray-200")?) { "turbo " (self.turbo_version) }
            (self.execution
                .as_ref()
                .map(|e| e.render_html(&mut tailwind, self.packages.len()))
                .transpose()?.unwrap_or_default())
        }
        };

        Ok(html! {
            html lang="en" {
                (DOCTYPE)
                style {
                    (tailwind.bundle()?)
                }
                head {
                    meta charset="utf-8";
                    meta name="viewport" content="width=device-width, initial-scale=1.0";
                    title { "Turborepo" }
                }
                (body)
            }
        }
        .into_string())
    }
}

fn trace(tailwind: &mut TailwindBuilder, class: &'static str) -> Result<&'static str, Error> {
    tailwind.trace(class, false)?;
    Ok(class)
}

impl<'a> ExecutionSummary<'a> {
    pub fn render_html(
        &self,
        tailwind: &mut TailwindBuilder,
        packages: usize,
    ) -> Result<Markup, Error> {
        Ok(html! {
            div {
                h1 class=(trace(tailwind, "text-2xl")?) {
                    "Ran "
                    code {
                        (self.command)
                    }
                    " in "
                    (packages)
                    " packages"
                }
            }
        })
    }
}
