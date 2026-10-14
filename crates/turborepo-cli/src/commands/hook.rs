//! `turbo hook <shell>`: prompt-time activation of the tools `turbo setup`
//! recorded in this user's state directory. Runs in the invoking global binary,
//! never a repository-local turbo, and never resolves, downloads, installs, or
//! writes locks or user state. Out-of-date tools stay active with a notice;
//! `turbo run` stays strict.

use std::{
    fmt::Write as _,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use turbopath::AbsoluteSystemPath;
use turborepo_turbo_json::RawTurboJson;

use crate::{cli::HookShell, commands::setup::trust::TrustStore};

/// Exported so nested shells agree on what the hook added. Internal only.
const STATE: &str = "__TURBO_HOOK_STATE";
const SEPARATOR: char = ':';

const BASH: &str = r#"_turbo_hook() {
  local previous_exit_status=$?
  eval "$(@EXE@ hook bash --export)"
  return $previous_exit_status
}
if [[ ";${PROMPT_COMMAND[*]:-};" != *";_turbo_hook;"* ]]; then
  if [[ "$(declare -p PROMPT_COMMAND 2>&1)" == "declare -a"* ]]; then
    PROMPT_COMMAND=(_turbo_hook "${PROMPT_COMMAND[@]}")
  else
    PROMPT_COMMAND="_turbo_hook${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
  fi
fi
"#;

const ZSH: &str = r#"_turbo_hook() {
  eval "$(@EXE@ hook zsh --export)"
}
typeset -ag precmd_functions
if (( ! ${precmd_functions[(I)_turbo_hook]} )); then
  precmd_functions=(_turbo_hook $precmd_functions)
fi
typeset -ag chpwd_functions
if (( ! ${chpwd_functions[(I)_turbo_hook]} )); then
  chpwd_functions=(_turbo_hook $chpwd_functions)
fi
"#;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Status {
    /// Not a managed repository: the hook changes nothing.
    #[default]
    Off,
    Active,
    Stale,
    Missing,
}

/// `1;<root id>;<stamp>;<status>;<bin>`. The bin is last because it may
/// contain `;`.
#[derive(Debug, Default, PartialEq, Eq)]
struct State {
    root: String,
    stamp: String,
    status: Status,
    bin: Option<String>,
}

impl State {
    fn parse(value: &str) -> Option<Self> {
        let mut fields = value.splitn(5, ';');
        if fields.next()? != "1" {
            return None;
        }
        let root = fields.next()?.to_owned();
        let stamp = fields.next()?.to_owned();
        let status = match fields.next()? {
            "off" => Status::Off,
            "active" => Status::Active,
            "stale" => Status::Stale,
            "missing" => Status::Missing,
            _ => return None,
        };
        let bin = Some(fields.next()?).filter(|bin| !bin.is_empty());
        Some(Self {
            root,
            stamp,
            status,
            bin: bin.map(str::to_owned),
        })
    }

    fn encode(&self) -> String {
        let status = match self.status {
            Status::Off => "off",
            Status::Active => "active",
            Status::Stale => "stale",
            Status::Missing => "missing",
        };
        let bin = self.bin.as_deref().unwrap_or_default();
        format!("1;{};{};{status};{bin}", self.root, self.stamp)
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Changes {
    path: Option<String>,
    /// `Some(None)` unsets the state.
    state: Option<Option<String>>,
    notice: Option<String>,
}

pub(crate) fn run(shell: HookShell, export: bool) -> i32 {
    let output = if export {
        // Never rebuild a PATH this hook cannot represent exactly.
        let path = match std::env::var_os("PATH").map(|path| path.into_string()) {
            None => String::new(),
            Some(Ok(path)) => path,
            Some(Err(_)) => return 0,
        };
        let previous = std::env::var(STATE).ok().and_then(|s| State::parse(&s));
        let cwd = std::env::current_dir().ok();
        let base = dirs_next::data_local_dir();
        let changes = changes(previous, &path, cwd.as_deref(), base.as_deref());
        if let Some(notice) = &changes.notice {
            let _ = writeln!(std::io::stderr(), "turbo: {notice}");
        }
        render(shell, &changes)
    } else {
        // Embed this binary so prompts never resolve another `turbo` on PATH.
        let Some(exe) = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.into_os_string().into_string().ok())
        else {
            let _ = writeln!(
                std::io::stderr(),
                "turbo: cannot locate this turbo executable for the shell hook"
            );
            return 1;
        };
        let template = match shell {
            HookShell::Bash => BASH,
            HookShell::Zsh => ZSH,
        };
        template.replace("@EXE@", &quote(&exe))
    };
    match std::io::stdout().lock().write_all(output.as_bytes()) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

fn changes(
    previous: Option<State>,
    path: &str,
    cwd: Option<&Path>,
    base: Option<&Path>,
) -> Changes {
    let Some(root) = cwd.and_then(find_root) else {
        // Outside every repository: remove only what the hook added.
        return previous.map_or_else(Changes::default, |previous| Changes {
            path: previous.bin.map(|bin| remove(path, &bin)),
            state: Some(None),
            notice: None,
        });
    };
    let id = digest(root.as_os_str().as_encoded_bytes());
    let trust = base
        .ok_or_else(|| "cannot locate the user state directory".to_owned())
        .and_then(|base| TrustStore::new(base, &root).map_err(|e| e.to_string()));
    // Fast path: unchanged root and inputs need no verification.
    let mut stamp = Vec::new();
    for file in [
        "turbo.lock",
        "turbo.json",
        "turbo.jsonc",
        ".turbo/tools/manifest.json",
    ] {
        metadata(&mut stamp, &root.join(file));
    }
    if let Ok(trust) = &trust {
        metadata(&mut stamp, &trust.path());
    }
    let same_root = previous.as_ref().filter(|previous| previous.root == id);
    if same_root.is_some_and(|same| same.stamp == finish(&stamp, same.bin.as_deref())) {
        return Changes::default();
    }

    let (status, bin, reason) = if !enabled(&root) {
        (Status::Off, None, None)
    } else {
        match trust.and_then(|trust| trust.recorded().map_err(|e| e.to_string())) {
            Ok(Some(recorded)) => match recorded.bin.into_os_string().into_string() {
                Ok(bin) if !bin.contains(SEPARATOR) => {
                    let status = if recorded.stale {
                        Status::Stale
                    } else {
                        Status::Active
                    };
                    (status, Some(bin), None)
                }
                _ => (
                    Status::Missing,
                    None,
                    Some("its path cannot be added to PATH".to_owned()),
                ),
            },
            Ok(None) => (Status::Missing, None, None),
            Err(reason) => (Status::Missing, None, Some(reason)),
        }
    };

    let previous_bin = previous.as_ref().and_then(|p| p.bin.as_deref());
    let path = (previous_bin != bin.as_deref()).then(|| {
        let path = previous_bin.map_or_else(|| path.to_owned(), |old| remove(path, old));
        bin.as_deref()
            .map_or_else(|| path.clone(), |bin| prepend(&path, bin))
    });
    let entered_or_changed = previous
        .as_ref()
        .is_none_or(|p| p.root != id || p.status != status);
    let notice = entered_or_changed
        .then(|| notice(status, reason.as_deref()))
        .flatten();
    let state = State {
        root: id,
        stamp: finish(&stamp, bin.as_deref()),
        status,
        bin,
    };
    Changes {
        path,
        state: Some(Some(state.encode())),
        notice,
    }
}

fn notice(status: Status, reason: Option<&str>) -> Option<String> {
    match (status, reason) {
        (Status::Stale, _) => Some(
            "turbo.lock changed since `turbo setup`; keeping the previously installed tools \
             active. Run `turbo setup` to update them."
                .to_owned(),
        ),
        (Status::Missing, None) => Some(
            "this repository's managed tools are not installed. Run `turbo setup` to install them."
                .to_owned(),
        ),
        (Status::Missing, Some(reason)) => Some(format!(
            "cannot activate this repository's managed tools ({reason}). Run `turbo setup`."
        )),
        (Status::Off | Status::Active, _) => None,
    }
}

/// The nearest `turbo.lock` marks the repository. Like setup discovery, a
/// nested clone never borrows an enclosing repository's tools.
fn find_root(cwd: &Path) -> Option<PathBuf> {
    for dir in cwd.ancestors() {
        if fs::symlink_metadata(dir.join("turbo.lock")).is_ok() {
            return Some(dir.to_owned());
        }
        if fs::symlink_metadata(dir.join(".git")).is_ok() {
            return None;
        }
    }
    None
}

fn enabled(root: &Path) -> bool {
    let Ok(root) = AbsoluteSystemPath::from_std_path(root) else {
        return false;
    };
    turborepo_config::resolve_turbo_config_path(root)
        .ok()
        .and_then(|config| RawTurboJson::read(root, &config, true).ok().flatten())
        .and_then(|raw| raw.future_flags)
        .is_some_and(|flags| flags.as_inner().experimental_setup)
}

fn metadata(stamp: &mut Vec<u8>, path: &Path) {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        stamp.push(0);
        return;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        for value in [
            metadata.dev(),
            metadata.ino(),
            u64::from(metadata.mode()),
            metadata.size(),
            metadata.mtime() as u64,
            metadata.mtime_nsec() as u64,
            metadata.ctime() as u64,
            metadata.ctime_nsec() as u64,
        ] {
            stamp.extend(value.to_le_bytes());
        }
    }
    #[cfg(not(unix))]
    stamp.extend(format!("{:?}{:?}", metadata.len(), metadata.modified().ok()).as_bytes());
}

/// The activated bin directory is part of the stamp, so a changed shim set is
/// verified again.
fn finish(stamp: &[u8], bin: Option<&str>) -> String {
    let mut stamp = stamp.to_vec();
    if let Some(bin) = bin {
        metadata(&mut stamp, Path::new(bin));
    }
    digest(&stamp)
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))[..16].to_owned()
}

fn prepend(path: &str, bin: &str) -> String {
    if path.is_empty() {
        bin.to_owned()
    } else {
        format!("{bin}{SEPARATOR}{path}")
    }
}

/// Removes only the first matching entry, keeping any other PATH edits.
fn remove(path: &str, bin: &str) -> String {
    let mut entries: Vec<_> = path.split(SEPARATOR).collect();
    if let Some(index) = entries.iter().position(|entry| *entry == bin) {
        entries.remove(index);
    }
    entries.join(&SEPARATOR.to_string())
}

fn render(shell: HookShell, changes: &Changes) -> String {
    let mut out = String::new();
    match shell {
        HookShell::Bash | HookShell::Zsh => {
            if let Some(path) = &changes.path {
                let _ = writeln!(out, "export PATH={};", quote(path));
            }
            match &changes.state {
                Some(Some(state)) => {
                    let _ = writeln!(out, "export {STATE}={};", quote(state));
                }
                Some(None) => {
                    let _ = writeln!(out, "unset {STATE};");
                }
                None => {}
            }
        }
    }
    out
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn path_edits_round_trip_exactly_and_keep_user_changes() {
        for original in ["", "/usr/bin", "/a:/b", "/bin:/a:/bin", "/a::/b"] {
            assert_eq!(remove(&prepend(original, "/bin"), "/bin"), original);
        }
        assert_eq!(remove("/mine:/tools:/a", "/tools"), "/mine:/a");
        assert_eq!(remove("/a:/b", "/tools"), "/a:/b");
    }

    #[test]
    fn state_round_trips_and_rejects_unknown_formats() {
        let state = State {
            root: "abc".into(),
            stamp: "def".into(),
            status: Status::Stale,
            bin: Some("/repo;x/bin".into()),
        };
        assert_eq!(State::parse(&state.encode()), Some(state));
        assert_eq!(State::parse("2;a;b;active;/bin"), None);
        assert_eq!(State::parse("1;a;b;bogus;"), None);
    }

    #[cfg(unix)]
    #[test]
    fn unmanaged_and_untrusted_repositories_activate_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let outer = temp.path().join("outer");
        let inner = outer.join("inner");
        fs::create_dir_all(inner.join(".git")).unwrap();
        fs::write(outer.join("turbo.lock"), "{}").unwrap();
        // A nested clone never borrows the enclosing repository.
        assert_eq!(find_root(&inner), None);
        assert_eq!(find_root(&outer).unwrap(), outer);

        let outer = outer.canonicalize().unwrap();
        fs::write(outer.join("turbo.json"), "{}").unwrap();
        let off = changes(None, "/usr/bin", Some(&outer), Some(&home));
        assert!(off.path.is_none() && off.notice.is_none());
        let off_state = State::parse(off.state.clone().unwrap().as_deref().unwrap());
        // Unchanged inputs take the fast path: no output at all.
        assert_eq!(
            changes(off_state, "/usr/bin", Some(&outer), Some(&home)),
            Changes::default()
        );

        fs::write(
            outer.join("turbo.json"),
            r#"{"futureFlags":{"experimentalSetup":true}}"#,
        )
        .unwrap();
        let missing = changes(None, "/usr/bin", Some(&outer), Some(&home));
        assert!(missing.path.is_none());
        assert!(missing.notice.unwrap().contains("not installed"));
        let state = State::parse(missing.state.unwrap().as_deref().unwrap());
        assert_eq!(state.as_ref().unwrap().status, Status::Missing);
        assert_eq!(
            changes(state, "/usr/bin", Some(&outer), Some(&home)),
            Changes::default()
        );
    }

    #[test]
    fn leaving_removes_only_the_hook_entry() {
        let state = State {
            bin: Some("/repo/bin".into()),
            status: Status::Active,
            ..Default::default()
        };
        let temp = tempfile::tempdir().unwrap();
        let left = changes(
            Some(state),
            "/mine:/repo/bin:/usr/bin",
            Some(temp.path()),
            None,
        );
        assert_eq!(left.path.as_deref(), Some("/mine:/usr/bin"));
        assert_eq!(left.state, Some(None));
        assert_eq!(
            changes(None, "/usr/bin", Some(temp.path()), None),
            Changes::default()
        );
    }
}
