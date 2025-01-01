use std::{backtrace::Backtrace, env};

use itertools::Itertools;
use miette::{Diagnostic, SourceSpan};
use turbopath::AbsoluteSystemPathBuf;
use turborepo_ui::ColorConfig;

use super::Error;

// all arguments that result in a stdout that much be directly parsable and
// should not be paired with additional output (from the update notifier for
// example)
static TURBO_PURE_OUTPUT_ARGS: [&str; 6] = [
    "--json",
    "--dry",
    "--dry-run",
    "--dry=json",
    "--graph",
    "--dry-run=json",
];

static TURBO_SKIP_NOTIFIER_ARGS: [&str; 5] =
    ["--help", "--h", "--version", "--v", "--no-update-notifier"];

#[derive(Debug, thiserror::Error, Diagnostic)]
#[error("cannot have multiple `--cwd` flags in command")]
#[diagnostic(code(turbo::shim::multiple_cwd))]
pub struct MultipleCwd {
    #[backtrace]
    backtrace: Backtrace,
    #[source_code]
    args_string: String,
    #[label("first flag declared here")]
    flag1: Option<SourceSpan>,
    #[label("but second flag declared here")]
    flag2: Option<SourceSpan>,
    #[label("and here")]
    flag3: Option<SourceSpan>,
    // The user should get the idea after the first 4 examples.
    #[label("and here")]
    flag4: Option<SourceSpan>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ShimArgs {
    pub cwd: AbsoluteSystemPathBuf,
    pub invocation_dir: AbsoluteSystemPathBuf,
    pub skip_infer: bool,
    pub verbosity: usize,
    pub force_update_check: bool,
    pub remaining_turbo_args: Vec<String>,
    pub forwarded_args: Vec<String>,
    pub color: bool,
    pub no_color: bool,
}

impl ShimArgs {
    pub fn parse() -> Result<Self, Error> {
        let invocation_dir = AbsoluteSystemPathBuf::cwd()?;
        Self::parse_from_iter(invocation_dir, std::env::args())
    }

    fn parse_from_iter(
        invocation_dir: AbsoluteSystemPathBuf,
        args: impl Iterator<Item = String>,
    ) -> Result<Self, Error> {
        let mut cwd_flag_idx = None;
        let mut cwds = Vec::new();
        let mut skip_infer = false;
        let mut found_verbosity_flag = false;
        let mut verbosity = 0;
        let mut force_update_check = false;
        let mut remaining_turbo_args = Vec::new();
        let mut forwarded_args = Vec::new();
        let mut is_forwarded_args = false;
        let mut color = false;
        let mut no_color = false;

        let args = args.skip(1);
        for (idx, arg) in args.enumerate() {
            // We've seen a `--` and therefore we do no parsing
            if is_forwarded_args {
                forwarded_args.push(arg);
            } else if arg == "--skip-infer" {
                skip_infer = true;
            } else if arg == "--check-for-update" {
                force_update_check = true;
            } else if arg == "--" {
                // If we've hit `--` we've reached the args forwarded to tasks.
                is_forwarded_args = true;
            } else if arg == "--verbosity" {
                // If we see `--verbosity` we expect the next arg to be a number.
                remaining_turbo_args.push(arg);
                found_verbosity_flag = true
            } else if arg.starts_with("--verbosity=") || found_verbosity_flag {
                let verbosity_count = if found_verbosity_flag {
                    found_verbosity_flag = false;
                    &arg
                } else {
                    arg.strip_prefix("--verbosity=").unwrap_or("0")
                };

                verbosity = verbosity_count.parse::<usize>().unwrap_or(0);
                remaining_turbo_args.push(arg);
            } else if arg == "-v" || arg.starts_with("-vv") {
                verbosity = arg[1..].len();
                remaining_turbo_args.push(arg);
            } else if cwd_flag_idx.is_some() {
                // We've seen a `--cwd` and therefore add this to the cwds list along with
                // the index of the `--cwd` (*not* the value)
                cwds.push((
                    AbsoluteSystemPathBuf::from_unknown(&invocation_dir, arg),
                    idx - 1,
                ));
                cwd_flag_idx = None;
            } else if arg == "--cwd" {
                // If we see a `--cwd` we expect the next arg to be a path.
                cwd_flag_idx = Some(idx)
            } else if let Some(cwd_arg) = arg.strip_prefix("--cwd=") {
                // In the case where `--cwd` is passed as `--cwd=./path/to/foo`, that
                // entire chunk is a single arg, so we need to split it up.
                cwds.push((
                    AbsoluteSystemPathBuf::from_unknown(&invocation_dir, cwd_arg),
                    idx,
                ));
            } else if arg == "--color" {
                color = true;
            } else if arg == "--no-color" {
                no_color = true;
            } else {
                remaining_turbo_args.push(arg);
            }
        }

        if let Some(idx) = cwd_flag_idx {
            let (spans, args_string) =
                Self::get_spans_in_args_string(vec![idx], env::args().skip(1));

            return Err(Error::EmptyCwd {
                backtrace: Backtrace::capture(),
                args_string,
                flag_range: spans[0],
            });
        }

        if cwds.len() > 1 {
            let (indices, args_string) = Self::get_spans_in_args_string(
                cwds.iter().map(|(_, idx)| *idx).collect(),
                env::args().skip(1),
            );

            let mut flags = indices.into_iter();
            return Err(Error::MultipleCwd(Box::new(MultipleCwd {
                backtrace: Backtrace::capture(),
                args_string,
                flag1: flags.next(),
                flag2: flags.next(),
                flag3: flags.next(),
                flag4: flags.next(),
            })));
        }

        let cwd = cwds
            .pop()
            .map(|(cwd, _)| cwd)
            .unwrap_or_else(|| invocation_dir.clone());

        Ok(ShimArgs {
            cwd,
            invocation_dir,
            skip_infer,
            verbosity,
            force_update_check,
            remaining_turbo_args,
            forwarded_args,
            color,
            no_color,
        })
    }

    /// Takes a list of indices into a Vec of arguments, i.e. ["--graph", "foo",
    /// "--cwd"] and converts them into `SourceSpan`'s into the string of those
    /// arguments, i.e. "-- graph foo --cwd". Returns the spans and the args
    /// string
    fn get_spans_in_args_string(
        mut args_indices: Vec<usize>,
        args: impl Iterator<Item = impl Into<String>>,
    ) -> (Vec<SourceSpan>, String) {
        // Sort the indices to keep the invariant
        // that if i > j then output[i] > output[j]
        args_indices.sort();
        let mut indices_in_args_string = Vec::new();
        let mut i = 0;
        let mut current_args_string_idx = 0;

        for (idx, arg) in args.enumerate() {
            let Some(arg_idx) = args_indices.get(i) else {
                break;
            };

            let arg = arg.into();

            if idx == *arg_idx {
                indices_in_args_string.push((current_args_string_idx, arg.len()).into());
                i += 1;
            }
            current_args_string_idx += arg.len() + 1;
        }

        let args_string = env::args().skip(1).join(" ");

        (indices_in_args_string, args_string)
    }

    // returns true if any flags result in pure json output to stdout
    fn has_json_flags(&self) -> bool {
        self.remaining_turbo_args
            .iter()
            .any(|arg| TURBO_PURE_OUTPUT_ARGS.contains(&arg.as_str()))
    }

    // returns true if any flags should bypass the update notifier
    fn has_notifier_skip_flags(&self) -> bool {
        self.remaining_turbo_args
            .iter()
            .any(|arg| TURBO_SKIP_NOTIFIER_ARGS.contains(&arg.as_str()))
    }

    pub fn should_check_for_update(&self) -> bool {
        if self.force_update_check {
            return true;
        }

        if self.has_notifier_skip_flags() || self.has_json_flags() {
            return false;
        }

        true
    }

    pub fn color_config(&self) -> ColorConfig {
        if self.no_color {
            ColorConfig::new(true)
        } else if self.color {
            // Do our best to enable ansi colors, but even if the terminal doesn't support
            // still emit ansi escape sequences.
            Self::supports_ansi();
            ColorConfig::new(false)
        } else if Self::supports_ansi() {
            // If the terminal supports ansi colors, then we can infer if we should emit
            // colors
            ColorConfig::infer()
        } else {
            ColorConfig::new(true)
        }
    }

    #[cfg(windows)]
    fn supports_ansi() -> bool {
        // This call has the side effect of setting ENABLE_VIRTUAL_TERMINAL_PROCESSING
        // to true. https://learn.microsoft.com/en-us/windows/console/setconsolemode
        crossterm::ansi_support::supports_ansi()
    }

    #[cfg(not(windows))]
    fn supports_ansi() -> bool {
        true
    }
}

#[cfg(test)]
mod test {
    use miette::SourceSpan;
    use pretty_assertions::assert_eq;
    use test_case::test_case;
    use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

    use super::ShimArgs;

    #[test_case(vec![3], vec!["--graph", "foo", "--cwd", "apple"], vec![(18, 5).into()])]
    #[test_case(vec![0], vec!["--graph", "foo", "--cwd"], vec![(0, 7).into()])]
    #[test_case(vec![0, 2], vec!["--graph", "foo", "--cwd"], vec![(0, 7).into(), (12, 5).into()])]
    #[test_case(vec![], vec!["--cwd"], vec![])]
    fn test_get_indices_in_arg_string(
        arg_indices: Vec<usize>,
        args: Vec<&'static str>,
        expected_indices_in_arg_string: Vec<SourceSpan>,
    ) {
        let (indices_in_args_string, _) =
            ShimArgs::get_spans_in_args_string(arg_indices, args.into_iter());
        assert_eq!(indices_in_args_string, expected_indices_in_arg_string);
    }

    #[derive(Default)]
    struct ExpectedArgs {
        pub skip_infer: bool,
        pub verbosity: usize,
        pub force_update_check: bool,
        pub remaining_turbo_args: &'static [&'static str],
        pub forwarded_args: &'static [&'static str],
        pub color: bool,
        pub no_color: bool,
        pub relative_cwd: Option<&'static [&'static str]>,
    }

    impl ExpectedArgs {
        fn build(self, invocation_dir: &AbsoluteSystemPath) -> ShimArgs {
            let Self {
                skip_infer,
                verbosity,
                force_update_check,
                remaining_turbo_args,
                forwarded_args,
                color,
                no_color,
                relative_cwd,
            } = self;
            ShimArgs {
                cwd: relative_cwd.map_or_else(
                    || invocation_dir.to_owned(),
                    |components| invocation_dir.join_components(components),
                ),
                invocation_dir: invocation_dir.to_owned(),
                remaining_turbo_args: remaining_turbo_args
                    .iter()
                    .map(|arg| arg.to_string())
                    .collect(),
                forwarded_args: forwarded_args.iter().map(|arg| arg.to_string()).collect(),
                skip_infer,
                verbosity,
                force_update_check,
                color,
                no_color,
            }
        }
    }

    #[test_case(
        &["turbo"],
        ExpectedArgs {
            ..Default::default()
        }
        ; "no args"
    )]
    #[test_case(
        &["turbo", "-v"],
        ExpectedArgs {
            verbosity: 1,
            remaining_turbo_args: &["-v"],
            ..Default::default()
        }
        ; "verbosity count 1"
    )]
    #[test_case(
        &["turbo", "-vv"],
        ExpectedArgs {
            verbosity: 2,
            remaining_turbo_args: &["-vv"],
            ..Default::default()
        }
        ; "verbosity count 2"
    )]
    #[test_case(
        &["turbo", "--verbosity", "3"],
        ExpectedArgs {
            verbosity: 3,
            remaining_turbo_args: &["--verbosity", "3"],
            ..Default::default()
        }
        ; "verbosity flag 3"
    )]
    #[test_case(
        &["turbo", "--verbosity=3"],
        ExpectedArgs {
            verbosity: 3,
            remaining_turbo_args: &["--verbosity=3"],
            ..Default::default()
        }
        ; "verbosity equals 3"
    )]
    #[test_case(
        &["turbo", "--verbosity=3", "-vv"],
        ExpectedArgs {
            verbosity: 2,
            remaining_turbo_args: &["--verbosity=3", "-vv"],
            ..Default::default()
        }
        ; "multi verbosity"
    )]
    #[test_case(
        &["turbo", "--color"],
        ExpectedArgs {
            color: true,
            ..Default::default()
        }
        ; "color"
    )]
    #[test_case(
        &["turbo", "--no-color"],
        ExpectedArgs {
            no_color: true,
            ..Default::default()
        }
        ; "no color"
    )]
    #[test_case(
        &["turbo", "--no-color", "--color"],
        ExpectedArgs {
            color: true,
            no_color: true,
            ..Default::default()
        }
        ; "confused color"
    )]
    #[test_case(
        &["turbo", "--skip-infer"],
        ExpectedArgs {
            skip_infer: true,
            ..Default::default()
        }
        ; "skip infer"
    )]
    #[test_case(
        &["turbo", "--", "another", "--skip-infer"],
        ExpectedArgs {
            forwarded_args: &["another", "--skip-infer"],
            ..Default::default()
        }
        ; "forwarded args"
    )]
    #[test_case(
        &["turbo", "--check-for-update"],
        ExpectedArgs {
            force_update_check: true,
            ..Default::default()
        }
        ; "check for update"
    )]
    #[test_case(
        &["turbo", "--check-for-update=true"],
        ExpectedArgs {
            force_update_check: false,
            remaining_turbo_args: &["--check-for-update=true"],
            ..Default::default()
        }
        ; "check for update value"
    )]
    #[test_case(
        &["turbo", "--cwd", "another-dir"],
        ExpectedArgs {
            relative_cwd: Some(&["another-dir"]),
            ..Default::default()
        }
        ; "cwd value"
    )]
    #[test_case(
        &["turbo", "--cwd=another-dir"],
        ExpectedArgs {
            relative_cwd: Some(&["another-dir"]),
            ..Default::default()
        }
        ; "cwd equals"
    )]
    fn test_shim_parsing(args: &[&str], expected: ExpectedArgs) {
        let cwd = AbsoluteSystemPathBuf::new(if cfg!(windows) {
            "Z:\\some\\dir"
        } else {
            "/some/dir"
        })
        .unwrap();
        let expected = expected.build(&cwd);
        let actual = ShimArgs::parse_from_iter(cwd, args.iter().map(|s| s.to_string())).unwrap();
        assert_eq!(expected, actual);
    }
}
