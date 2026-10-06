//! Exact native preflight, before any normal CLI/shim initialization.

use std::{ffi::OsStr, io::Write};

use turborepo_shim::capabilities::{Capabilities, QUERY_FLAG};

#[derive(Debug, PartialEq, Eq)]
enum Request {
    Query,
    Invalid,
}

fn request<T: AsRef<OsStr>>(args: impl IntoIterator<Item = T>) -> Option<Request> {
    let mut args = args.into_iter().skip(1);
    let first = args.next()?;
    let first = first.as_ref();
    if first == QUERY_FLAG {
        return Some(if args.next().is_none() {
            Request::Query
        } else {
            Request::Invalid
        });
    }
    // A value on this boolean internal flag is also rejected without entering
    // normal startup. Other commands and flags (including forwarded task args)
    // retain their existing routing.
    if first
        .to_str()
        .and_then(|s| s.strip_prefix(QUERY_FLAG))
        .is_some_and(|suffix| suffix.starts_with('='))
    {
        return Some(Request::Invalid);
    }
    None
}

pub(crate) fn run_query<T: AsRef<OsStr>>(args: impl IntoIterator<Item = T>) -> Option<i32> {
    match request(args)? {
        Request::Invalid => {
            let _ = writeln!(
                std::io::stderr().lock(),
                "error: {QUERY_FLAG} must be used alone"
            );
            Some(1)
        }
        Request::Query => {
            let bytes = match Capabilities::unsupported(crate::get_version()).encode() {
                Ok(bytes) => bytes,
                Err(err) => {
                    let _ = writeln!(std::io::stderr().lock(), "error: {err}");
                    return Some(1);
                }
            };
            // Do not use println!: a broken stdout pipe must not panic and enter
            // the crash-reporting path for this otherwise side-effect-free query.
            if std::io::stdout().lock().write_all(&bytes).is_err() {
                let _ = writeln!(
                    std::io::stderr().lock(),
                    "error: cannot write managed-run capability response"
                );
                return Some(1);
            }
            Some(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_exact_standalone_native_query_is_accepted() {
        assert_eq!(request(["turbo", QUERY_FLAG]), Some(Request::Query));
        for tail in ["run", "setup", "--", "--help", "--profile=out", QUERY_FLAG] {
            assert_eq!(request(["turbo", QUERY_FLAG, tail]), Some(Request::Invalid));
        }
        assert_eq!(
            request(["turbo", "--__internal-managed-run-capabilities=true"]),
            Some(Request::Invalid)
        );
    }

    #[test]
    fn unrelated_invocations_and_task_arguments_do_not_become_queries() {
        for args in [
            vec![],
            vec!["turbo"],
            vec!["turbo", "setup"],
            vec!["turbo", "--help"],
            vec!["turbo", "run", "build", "--", QUERY_FLAG],
            vec!["turbo", "--cwd", QUERY_FLAG, "build"],
            vec!["turbo", "--skip-infer", "build"],
            vec!["turbo", "__internal-managed-run-capabilities"],
        ] {
            assert_eq!(request(args), None);
        }
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_tail_is_rejected_without_unicode_argument_parsing() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};
        assert_eq!(
            request([
                OsString::from("turbo"),
                OsString::from(QUERY_FLAG),
                OsString::from_vec(vec![0xff])
            ]),
            Some(Request::Invalid)
        );
    }
}
