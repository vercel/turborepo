use human_panic::report::{Method, Report};
use turborepo_lib::get_version;

pub fn panic_handler(panic_info: &std::panic::PanicInfo) {
    let cause = panic_info
        .message()
        .map(ToString::to_string)
        .unwrap_or_else(|| "Unknown".to_string());

    let explanation = match panic_info.location() {
        Some(location) => format!("file '{}' at line {}\n", location.file(), location.line()),
        None => "unknown.".to_string(),
    };

    let report = Report::new("turbo", get_version(), Method::Panic, explanation, cause);

    let report_message = match report.persist() {
        Ok(f) => {
            format!(
                "A report has been written to {}\n
Please open an issue at https://github.com/vercel/turbo/issues/new/choose \
                 and include this file",
                f.display()
            )
        }
        Err(e) => {
            format!(
                "An error has occurred while attempting to write a report.\n
Please open an issue at \
                 https://github.com/vercel/turbo/issues/new/choose and include the following \
                 error in your issue: {}",
                e
            )
        }
    };

    eprintln!(
        "Oops! Turbo has crashed.
         
{}",
        report_message
    );
}
