use human_panic::report::{Method, Report};

use crate::get_version;

const OPEN_ISSUE_MESSAGE: &str =
    "Please open an issue at https://github.com/vercel/turborepo/issues/new/choose";

pub fn panic_handler(panic_info: &std::panic::PanicHookInfo) {
    let cause = panic_info.to_string();

    let explanation = match panic_info.location() {
        Some(location) => format!("file '{}' at line {}\n", location.file(), location.line()),
        None => "unknown.".to_string(),
    };

    let report = Report::new("turbo", get_version(), Method::Panic, explanation, cause);
    // If we're in CI we don't persist the backtrace to a temp file as this is hard
    // to retrieve.
    let should_persist = !turborepo_ci::is_ci() && turborepo_ci::Vendor::infer().is_none();

    let report_message = if should_persist {
        match report.persist() {
            Ok(f) => {
                format!(
                    "A report has been written to {}\n\n{OPEN_ISSUE_MESSAGE} and include this file",
                    f.display()
                )
            }
            Err(e) => {
                format!(
                    "An error has occurred while attempting to write a \
                     report.\n\n{OPEN_ISSUE_MESSAGE} and include the following error in your \
                     issue: {}",
                    e
                )
            }
        }
    } else if let Some(backtrace) = report.serialize() {
        format!(
            "Caused by \n{backtrace}\n\n{OPEN_ISSUE_MESSAGE} and include this message in your \
             issue"
        )
    } else {
        format!(
            "Unable to serialize backtrace.\n\n{OPEN_ISSUE_MESSAGE} and include this message in \
             your issue"
        )
    };

    eprintln!(
        "Oops! Turbo has crashed.

{}",
        report_message
    );
}
