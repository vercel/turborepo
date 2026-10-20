//! Visitor support types and traits.
//!
//! This module provides shared types and trait definitions for the task graph
//! visitor. The concrete `Visitor` implementation lives in
//! `turborepo-task-graph`, but these abstractions allow for decoupling and
//! testing.

use tokio::sync::mpsc;
use turborepo_task_id::TaskId;
use turborepo_types::{StopExecution, TaskDefinition};

/// Trait for providing task execution messages from the engine.
pub trait EngineProvider: Send + Sync {
    /// The error type for engine execution.
    type Error: std::error::Error + Send;

    /// Returns an iterator over all task IDs in the engine.
    fn tasks(&self) -> Box<dyn Iterator<Item = TaskId<'static>> + '_>;

    /// Returns the task definition for a given task ID.
    fn task_definition(&self, task_id: &TaskId) -> Option<&TaskDefinition>;

    /// Returns the dependencies for a given task ID.
    fn dependencies(&self, task_id: &TaskId) -> Option<Vec<TaskId<'static>>>;
}

/// Callback for task completion.
pub type TaskCallback = tokio::sync::oneshot::Sender<Result<(), StopExecution>>;

/// Message from the engine containing task info and completion callback.
pub struct EngineMessage {
    pub task_id: TaskId<'static>,
    pub callback: TaskCallback,
}

/// Trait for engine execution.
pub trait EngineExecutor: Send {
    /// The error type for execution.
    type Error: std::error::Error + Send + 'static;

    /// Execute the engine and send task messages to the provided sender.
    fn execute(
        self,
        concurrency: usize,
        sender: mpsc::Sender<EngineMessage>,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send;
}

/// Check for Turbo invocations that can execute tasks recursively.
///
/// This is a conservative check, not a shell interpreter. For shell constructs
/// we cannot classify, use a conservative word-based guard.
pub fn command_invokes_turbo(command: &str) -> bool {
    let fallback = || contains_turbo_word(command);
    let Some(commands) = split_commands(command) else {
        return fallback();
    };
    commands.into_iter().any(|command| {
        shell_words::split(command).map_or_else(|_| fallback(), |words| invokes_tasks(&words))
    })
}

fn contains_turbo_word(command: &str) -> bool {
    command
        .split(|ch: char| !ch.is_alphanumeric() && !matches!(ch, '_' | '-' | '.' | '/' | '\\'))
        .any(is_turbo_binary)
}

fn is_turbo_binary(program: &str) -> bool {
    let binary = program.rsplit(['/', '\\']).next().unwrap_or(program);
    matches!(binary, "turbo" | "turbo.exe" | "turbo.cmd")
        || (cfg!(windows)
            && ["turbo", "turbo.exe", "turbo.cmd"]
                .iter()
                .any(|name| binary.eq_ignore_ascii_case(name)))
}

/// Split unquoted shell operators before parsing words, so `echo turbo` and
/// `echo ready&&turbo run build` have different executable positions.
fn split_commands(command: &str) -> Option<Vec<&str>> {
    let mut commands = Vec::new();
    let mut start = 0;
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in command.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match (quote, ch) {
            (Some('\''), '\'') | (Some('"'), '"') => quote = None,
            (Some('\''), _) => {}
            // Expansions may execute commands or change the executable name.
            (_, '$' | '`') | (None, '<' | '>' | '#' | '^' | '%') => return None,
            // CMD does not use single quotes for quoting.
            (None, '\'') if cfg!(windows) => return None,
            (_, '%' | '!') if cfg!(windows) => return None,
            (None, '\'' | '"') => quote = Some(ch),
            (_, '\\') if cfg!(windows) => return None,
            (_, '\\') => escaped = true,
            (Some(_), _) => {}
            (None, ';' | '&' | '|' | '(' | ')' | '\n') => {
                commands.push(&command[start..index]);
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    if quote.is_some() || escaped {
        return None;
    }
    commands.push(&command[start..]);
    Some(commands)
}

fn invokes_tasks(words: &[String]) -> bool {
    let fallback = || words.iter().any(|word| contains_turbo_word(word));
    let mut args = words.iter().map(String::as_str).peekable();
    loop {
        while args.peek().is_some_and(|word| {
            word.split_once('=').is_some_and(|(name, _)| {
                !name.is_empty()
                    && name.chars().enumerate().all(|(index, ch)| {
                        ch == '_' || ch.is_ascii_alphabetic() || (index > 0 && ch.is_ascii_digit())
                    })
            })
        }) {
            args.next();
        }
        let Some(program) = args.next() else {
            return false;
        };
        // Windows resolves executable names without case sensitivity. Leave
        // the arguments alone: Turbo's subcommand names are case-sensitive.
        let normalized_program = cfg!(windows).then(|| program.to_ascii_lowercase());
        let program = normalized_program.as_deref().unwrap_or(program);
        match program {
            "!" | "if" | "then" | "else" | "do" | "while" | "until" | "{" => continue,
            "npx" => {
                while args.peek().is_some_and(|arg| arg.starts_with('-')) {
                    match args.next() {
                        Some("--") => break,
                        Some("-y" | "--yes" | "--no-install") => {}
                        _ => return fallback(),
                    }
                }
            }
            "pnpm" | "npm" | "yarn" | "bun" => {
                // Wrapper names differ between package managers. Unknown or
                // version-dependent commands can select a package script.
                if args.peek().is_some_and(|arg| {
                    matches!(
                        (program, *arg),
                        ("pnpm", "exec" | "dlx") | ("npm" | "yarn", "exec") | ("bun", "x")
                    )
                }) {
                    args.next();
                } else {
                    return fallback();
                }
                if args.peek() == Some(&"--") {
                    args.next();
                }
                if !args.peek().is_some_and(|arg| {
                    if cfg!(windows) {
                        arg.eq_ignore_ascii_case("turbo")
                    } else {
                        *arg == "turbo"
                    }
                }) {
                    return fallback();
                }
            }
            "echo" | "printf" => return false,
            _ => {
                if !is_turbo_binary(program) {
                    return fallback();
                }
                while args.peek().is_some_and(|arg| arg.starts_with('-')) {
                    match args.next() {
                        Some("--help" | "-h" | "--version") => return false,
                        Some("--cwd") => {
                            args.next();
                        }
                        Some(arg) if arg.starts_with("--cwd=") => {}
                        _ => return true,
                    }
                }
                // These inspect the repository without dispatching its tasks.
                // Keep run, watch, shorthand tasks, exec, and unknown commands
                // guarded rather than treating every subcommand as safe.
                return !args.next().is_some_and(|arg| {
                    matches!(arg, "boundaries" | "query" | "ls" | "info" | "bin")
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_invocations_remain_guarded() {
        for command in [
            "turbo",
            "turbo build",
            "turbo run build",
            "turbo watch build",
            "npx turbo build",
            "npx --yes turbo run build",
            "pnpm exec turbo run build",
            "npm exec -- turbo build",
            "yarn turbo build",
            "yarn turbo boundaries",
            "pnpm turbo boundaries",
            "bun turbo boundaries",
            "bun x turbo build",
            "echo ready && turbo run build",
            "echo ready&&turbo run build",
            "turbo boundaries;turbo build",
            "turbo boundaries && turbo run build",
            "\"turbo\" run build",
            "./node_modules/.bin/turbo run build",
            "turbo.cmd run build",
            "FOO=1 turbo run build",
            "cross-env FOO=1 turbo run build",
            "custom-launcher turbo run build",
            "turbo --cwd boundaries build",
            "turbo --unknown boundaries build",
            "turbo exec -- turbo run build",
            "if turbo build; then echo hi; fi",
            "sh -c 'turbo run build'",
            "custom-launcher './node_modules/.bin/turbo' run build",
        ] {
            assert!(command_invokes_turbo(command), "{command}");
        }
    }

    #[test]
    fn printing_and_inspection_do_not_run_tasks() {
        for command in [
            "echo turbo",
            "printf turbo",
            "echo \"turbo run build\"",
            "echo \"ready && turbo build\"",
            "echo turbo && echo safe",
            "turbo boundaries",
            "pnpm exec turbo boundaries",
            "npx --yes turbo boundaries",
            "npm exec -- turbo boundaries",
            "yarn exec turbo boundaries",
            "bun x turbo boundaries",
            "turbo --cwd . boundaries",
            "turbo --cwd=. boundaries",
            "turbo query \"query { packages { items { name } } }\"",
            "turbo ls",
            "turbo info",
            "turbo bin",
            "turbo --help",
            "turbo --version",
            "turbo boundaries && echo turbo",
            "turbopack",
            "myturbo",
            "",
        ] {
            assert!(!command_invokes_turbo(command), "{command}");
        }
    }

    #[test]
    fn execution_wrappers_are_specific_to_each_package_manager() {
        for command in [
            "pnpm exec turbo boundaries",
            "pnpm dlx turbo boundaries",
            "npm exec -- turbo boundaries",
            "yarn exec turbo boundaries",
            "bun x turbo boundaries",
        ] {
            assert!(!command_invokes_turbo(command), "{command}");
        }
        for command in [
            "pnpm x turbo boundaries",
            "npm x turbo boundaries",
            "npm dlx turbo boundaries",
            "yarn x turbo boundaries",
            // Yarn Classic can select a package script named `dlx`.
            "yarn dlx turbo boundaries",
            "bun dlx turbo boundaries",
            "bun exec turbo boundaries",
        ] {
            assert!(command_invokes_turbo(command), "{command}");
        }
    }

    #[test]
    fn unsupported_shell_syntax_is_checked_conservatively() {
        for command in [
            "echo $(turbo run build)",
            "echo `turbo run build`",
            "echo turbo > output.txt",
            "echo \"turbo",
            "echo turbo # comment",
            "echo turbo ^& other-command",
            "echo %COMMAND% turbo",
        ] {
            assert!(command_invokes_turbo(command), "{command}");
        }
    }

    #[test]
    #[cfg(windows)]
    fn cmd_invocations_and_conservative_fallbacks() {
        for command in [
            "TURBO run build",
            "TURBO BOUNDARIES",
            "TuRbO.EXE run build",
            "PNPM exec TURBO run build",
            "echo ready&turbo run build",
            "echo ready|turbo run build",
            "set FOO=1&&turbo run build",
            "cmd /c \"turbo run build\"",
            "call turbo run build",
            "\"C:\\tools\\turbo.exe\" run build",
            "echo 'ready&turbo run build'",
            "echo turbo ^& echo safe",
            "echo \"%RUNNER% turbo\"",
            "echo \"!RUNNER! turbo\"",
        ] {
            assert!(command_invokes_turbo(command), "{command}");
        }
        for command in [
            "ECHO turbo",
            "TURBO boundaries",
            "PNPM exec TURBO boundaries",
            "echo \"ready & turbo run build\"",
            "turbo.exe boundaries",
            "turbo.cmd boundaries",
            "turbo boundaries & echo turbo",
        ] {
            assert!(!command_invokes_turbo(command), "{command}");
        }
    }

    #[test]
    #[cfg(not(windows))]
    fn shell_quotes_and_line_continuations() {
        for command in [
            "echo 'turbo'",
            "echo 'ready; turbo run build'",
            "turbo \\\n boundaries",
        ] {
            assert!(!command_invokes_turbo(command), "{command}");
        }
        assert!(command_invokes_turbo("'turbo' run build"));
    }
}
