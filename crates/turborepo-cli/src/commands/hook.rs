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
const SEPARATOR: char = if cfg!(windows) { ';' } else { ':' };

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

// Redefining the function replaces its handlers, so sourcing twice is safe.
const FISH: &str = r#"function _turbo_hook --on-event fish_prompt --on-variable PWD
  @EXE@ hook fish --export | source
end
"#;

// Windows PowerShell 5.1 and PowerShell 7 have no directory hook, so wrap the
// prompt once. The original prompt runs first so it still sees the user's `$?`.
const POWERSHELL: &str = r#"if ($null -eq $global:__TurboHookPrompt) {
  $global:__TurboHookPrompt = $function:prompt
  function global:prompt {
    $turboPrompt = & $global:__TurboHookPrompt
    $turboExitCode = $global:LASTEXITCODE
    $turboHook = (& @EXE@ hook powershell --export) -join "`n"
    if ($turboHook) { Invoke-Expression $turboHook }
    $global:LASTEXITCODE = $turboExitCode
    $turboPrompt
  }
}
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

/// Runtime variables the hook may set beside PATH (see `runtime`).
const RUNTIME: [&str; 2] = ["GOROOT", "GOTOOLCHAIN"];

/// Runtime variables by name, with a value or `None` for unset.
type Vars = Vec<(String, Option<String>)>;

/// `1;<root id>;<stamp>;<status>;<bin>`, or with runtime variables
/// `2;<root id>;<stamp>;<status>;<saved>;<bin>`. The bin is last because it
/// may contain `;`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct State {
    root: String,
    stamp: String,
    status: Status,
    /// Each runtime variable the hook set, with its value from before the
    /// hook first set it (`None` when it was unset). Encoded as `,`-separated
    /// `NAME` or `NAME=<hex>` entries.
    saved: Vars,
    bin: Option<String>,
}

impl State {
    fn parse(value: &str) -> Option<Self> {
        let (version, rest) = value.split_once(';')?;
        let saved = match version {
            "1" => false,
            "2" => true,
            _ => return None,
        };
        let mut fields = rest.splitn(if saved { 5 } else { 4 }, ';');
        let root = fields.next()?.to_owned();
        let stamp = fields.next()?.to_owned();
        let status = match fields.next()? {
            "off" => Status::Off,
            "active" => Status::Active,
            "stale" => Status::Stale,
            "missing" => Status::Missing,
            _ => return None,
        };
        let saved = match saved {
            true => fields
                .next()?
                .split(',')
                .map(|entry| {
                    let (name, value) = match entry.split_once('=') {
                        Some((name, hex)) => (name, Some(unhex(hex)?)),
                        None => (entry, None),
                    };
                    RUNTIME.contains(&name).then(|| (name.to_owned(), value))
                })
                .collect::<Option<_>>()?,
            false => Vec::new(),
        };
        let bin = Some(fields.next()?).filter(|bin| !bin.is_empty());
        Some(Self {
            root,
            stamp,
            status,
            saved,
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
        let (root, stamp) = (&self.root, &self.stamp);
        if self.saved.is_empty() {
            return format!("1;{root};{stamp};{status};{bin}");
        }
        let saved: Vec<_> = self
            .saved
            .iter()
            .map(|(name, value)| match value {
                Some(value) => format!("{name}={}", hex(value)),
                None => name.clone(),
            })
            .collect();
        format!("2;{root};{stamp};{status};{};{bin}", saved.join(","))
    }
}

fn hex(value: &str) -> String {
    value.bytes().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(hex: &str) -> Option<String> {
    if !hex.len().is_multiple_of(2) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect::<Option<Vec<_>>>()?;
    String::from_utf8(bytes).ok()
}

/// What the hook sets beside PATH for a recorded installation. Windows exports
/// `go.exe` as a hard link that finds its tree only through GOROOT, so it gets
/// the activation's GOROOT (the Store's `tools/go` beside `bin`) and
/// GOTOOLCHAIN, as `turbo run` and `turbo exec` do. Unix exports are symlinks
/// that find their own tree, and their hook output is unchanged.
fn runtime(bin: &str, go: bool) -> Vec<(&'static str, String)> {
    let goroot = Path::new(bin).parent().and_then(|generation| {
        generation
            .join("tools")
            .join("go")
            .to_str()
            .map(str::to_owned)
    });
    match goroot {
        Some(goroot) if cfg!(windows) && go => std::iter::once(("GOROOT", goroot))
            .chain(
                turborepo_setup::go_provision::RUNTIME_ENV
                    .iter()
                    .map(|(name, value)| (*name, (*value).to_owned())),
            )
            .collect(),
        _ => Vec::new(),
    }
}

/// Runtime variable edits from the previous activation to `wanted`, and what
/// to restore later. Like PATH edits, a variable the user changed since the
/// hook set it is left alone.
fn runtime_changes(
    previous: Option<&State>,
    wanted: &[(&'static str, String)],
    vars: &impl Fn(&str) -> Option<String>,
) -> (Vars, Vars) {
    let saved = previous.map_or(&[][..], |previous| &previous.saved[..]);
    let set = previous
        .and_then(|previous| previous.bin.as_deref())
        .map(|bin| runtime(bin, true))
        .unwrap_or_default();
    let ours = |name: &str| set.iter().find(|(n, _)| *n == name).map(|(_, v)| v.clone());
    let mut edits = Vec::new();
    for (name, before) in saved {
        if !wanted.iter().any(|(n, _)| *n == name.as_str()) && vars(name) == ours(name) {
            edits.push((name.clone(), before.clone()));
        }
    }
    let mut keep = Vec::new();
    for (name, value) in wanted {
        let before = saved.iter().find(|(n, _)| n.as_str() == *name);
        if before.is_none() || ours(name).as_ref() != Some(value) {
            edits.push((name.to_string(), Some(value.clone())));
        }
        let before = before.map_or_else(|| vars(name), |(_, before)| before.clone());
        keep.push((name.to_string(), before));
    }
    (edits, keep)
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Changes {
    path: Option<String>,
    /// Runtime variables to set, or unset when `None`.
    env: Vars,
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
        let vars = |name: &str| std::env::var(name).ok();
        let changes = changes(previous, &path, cwd.as_deref(), base.as_deref(), vars);
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
            HookShell::Fish => FISH,
            HookShell::PowerShell => POWERSHELL,
        };
        template.replace("@EXE@", &quote(shell, &exe))
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
    vars: impl Fn(&str) -> Option<String>,
) -> Changes {
    let Some(root) = cwd.and_then(find_root) else {
        // Outside every repository: remove only what the hook added.
        return previous.map_or_else(Changes::default, |previous| Changes {
            path: previous.bin.as_deref().map(|bin| remove(path, bin)),
            env: runtime_changes(Some(&previous), &[], &vars).0,
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

    // Setup records trusted roots on Unix and Windows x64 (the only Windows
    // host it provisions), so `turbo setup` cannot help elsewhere: stay off
    // rather than print a notice. Without turbo.lock only this user's `turbo
    // setup --no-lock` record opts a clone in, so an unrecorded lockless clone
    // stays off quietly too.
    let locked = fs::symlink_metadata(root.join("turbo.lock")).is_ok();
    let qualified = cfg!(any(unix, all(windows, target_arch = "x86_64")));
    let (status, bin, go, reason) = if !qualified || !enabled(&root) {
        (Status::Off, None, false, None)
    } else {
        match trust.and_then(|trust| trust.recorded().map_err(|e| e.to_string())) {
            Ok(Some(recorded)) => {
                // cmd.exe does not resolve a `.cmd` export's `%~dp0..` under
                // a verbatim `\\?\` PATH entry; use the spelling activation
                // gives `turbo run`.
                #[cfg(windows)]
                let bin = dunce::simplified(&recorded.bin).to_path_buf();
                #[cfg(not(windows))]
                let bin = recorded.bin;
                match bin.into_os_string().into_string() {
                    Ok(bin) if !bin.contains(SEPARATOR) => {
                        let status = if recorded.stale {
                            Status::Stale
                        } else {
                            Status::Active
                        };
                        (status, Some(bin), recorded.go, None)
                    }
                    _ => (
                        Status::Missing,
                        None,
                        false,
                        Some("its path cannot be added to PATH".to_owned()),
                    ),
                }
            }
            _ if !locked => (Status::Off, None, false, None),
            Ok(None) => (Status::Missing, None, false, None),
            Err(reason) => (Status::Missing, None, false, Some(reason)),
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
    let wanted = bin.as_deref().map_or_else(Vec::new, |bin| runtime(bin, go));
    let (env, saved) = runtime_changes(previous.as_ref(), &wanted, &vars);
    let state = State {
        root: id,
        stamp: finish(&stamp, bin.as_deref()),
        status,
        saved,
        bin,
    };
    Changes {
        path,
        env,
        state: Some(Some(state.encode())),
        notice,
    }
}

/// One prompt's PowerShell export and the state it leaves, for setup's native
/// Windows fixtures.
#[cfg(all(test, windows))]
pub(crate) fn powershell_for_tests(
    state: Option<&str>,
    path: &str,
    cwd: &Path,
    base: &Path,
    vars: impl Fn(&str) -> Option<String>,
) -> (String, Option<String>) {
    let previous = state.and_then(State::parse);
    let changes = changes(previous, path, Some(cwd), Some(base), vars);
    (
        render(HookShell::PowerShell, &changes),
        changes.state.flatten(),
    )
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

/// The nearest `turbo.lock`, or the local inventory `turbo setup --no-lock`
/// leaves instead, marks the repository. Like setup discovery, a nested clone
/// never borrows an enclosing repository's tools.
fn find_root(cwd: &Path) -> Option<PathBuf> {
    for dir in cwd.ancestors() {
        let inventory = dir.join(".turbo").join("tools").join("manifest.json");
        if fs::symlink_metadata(dir.join("turbo.lock")).is_ok()
            || fs::symlink_metadata(inventory).is_ok()
        {
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
    let mut set = |name: &str, value: Option<&str>| {
        let _ = match (shell, value.map(|value| quote(shell, value))) {
            (HookShell::Bash | HookShell::Zsh, Some(value)) => {
                writeln!(out, "export {name}={value};")
            }
            // fish splits PATH on `:` like the environment does.
            (HookShell::Fish, Some(value)) => writeln!(out, "set -gx {name} {value};"),
            (HookShell::PowerShell, Some(value)) => writeln!(out, "$env:{name} = {value}"),
            (HookShell::Bash | HookShell::Zsh, None) => writeln!(out, "unset {name};"),
            (HookShell::Fish, None) => writeln!(out, "set -e {name};"),
            (HookShell::PowerShell, None) => writeln!(
                out,
                "Remove-Item -LiteralPath Env:{name} -ErrorAction SilentlyContinue"
            ),
        };
    };
    if let Some(path) = &changes.path {
        set("PATH", Some(path));
    }
    for (name, value) in &changes.env {
        set(name, value.as_deref());
    }
    if let Some(state) = &changes.state {
        set(STATE, state.as_deref());
    }
    out
}

fn quote(shell: HookShell, value: &str) -> String {
    let mut quoted = String::from("'");
    for c in value.chars() {
        match (shell, c) {
            (HookShell::Bash | HookShell::Zsh, '\'') => quoted.push_str(r"'\''"),
            (HookShell::Fish, '\'' | '\\') => {
                quoted.push('\\');
                quoted.push(c);
            }
            // PowerShell also closes single quotes with typographic quotes.
            (HookShell::PowerShell, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') => {
                quoted.push(c);
                quoted.push(c);
            }
            _ => quoted.push(c),
        }
    }
    quoted.push('\'');
    quoted
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    /// Writes test PATHs with `:` and converts them to the native separator.
    fn p(path: &str) -> String {
        path.replace(':', &SEPARATOR.to_string())
    }

    /// The hook in a shell that has none of the runtime variables set.
    fn changes(
        previous: Option<State>,
        path: &str,
        cwd: Option<&Path>,
        base: Option<&Path>,
    ) -> Changes {
        super::changes(previous, path, cwd, base, |_| None)
    }

    #[test]
    fn path_edits_round_trip_exactly_and_keep_user_changes() {
        for original in ["", "/usr/bin", "/a:/b", "/bin:/a:/bin", "/a::/b"] {
            let original = p(original);
            assert_eq!(remove(&prepend(&original, "/bin"), "/bin"), original);
        }
        assert_eq!(remove(&p("/mine:/tools:/a"), "/tools"), p("/mine:/a"));
        assert_eq!(remove(&p("/a:/b"), "/tools"), p("/a:/b"));
    }

    #[test]
    fn every_shell_quotes_and_applies_the_same_changes() {
        let changes = Changes {
            path: Some(r"/it's\here".into()),
            state: Some(None),
            ..Default::default()
        };
        assert_eq!(
            render(HookShell::Bash, &changes),
            "export PATH='/it'\\''s\\here';\nunset __TURBO_HOOK_STATE;\n"
        );
        assert_eq!(
            render(HookShell::Fish, &changes),
            "set -gx PATH '/it\\'s\\\\here';\nset -e __TURBO_HOOK_STATE;\n"
        );
        assert_eq!(
            render(HookShell::PowerShell, &changes),
            "$env:PATH = '/it''s\\here'\nRemove-Item -LiteralPath Env:__TURBO_HOOK_STATE \
             -ErrorAction SilentlyContinue\n"
        );
        assert_eq!(
            quote(HookShell::PowerShell, "a\u{2019}b"),
            "'a\u{2019}\u{2019}b'"
        );
        let state = Changes {
            state: Some(Some("1;x".into())),
            ..Default::default()
        };
        assert_eq!(
            render(HookShell::PowerShell, &state),
            "$env:__TURBO_HOOK_STATE = '1;x'\n"
        );
        // Runtime variables sit between PATH and the state; `None` unsets.
        let env = Changes {
            env: vec![
                ("GOROOT".into(), Some(r"C:\it's".into())),
                ("GOTOOLCHAIN".into(), None),
            ],
            ..Default::default()
        };
        assert_eq!(
            render(HookShell::PowerShell, &env),
            "$env:GOROOT = 'C:\\it''s'\nRemove-Item -LiteralPath Env:GOTOOLCHAIN -ErrorAction \
             SilentlyContinue\n"
        );
        assert_eq!(
            render(HookShell::Zsh, &env),
            "export GOROOT='C:\\it'\\''s';\nunset GOTOOLCHAIN;\n"
        );
    }

    #[test]
    fn state_round_trips_and_rejects_unknown_formats() {
        let mut state = State {
            root: "abc".into(),
            stamp: "def".into(),
            status: Status::Stale,
            saved: Vec::new(),
            bin: Some("/repo;x/bin".into()),
        };
        // Without runtime variables the state keeps its first format.
        assert!(state.encode().starts_with("1;"));
        assert_eq!(State::parse(&state.encode()), Some(state.clone()));
        state.saved = vec![
            ("GOROOT".into(), Some(r"C:\Go;x,y=z".into())),
            ("GOTOOLCHAIN".into(), None),
        ];
        assert!(state.encode().starts_with("2;"));
        assert_eq!(State::parse(&state.encode()), Some(state));
        // Only known variables with valid saved values are restored.
        for saved in ["PATH", "GOROOT=4", "GOROOT=+f", "GOROOT=zz", "GOROOT=ff"] {
            assert_eq!(State::parse(&format!("2;a;b;active;{saved};/bin")), None);
        }
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

    #[cfg(unix)]
    #[test]
    fn lockless_clones_activate_only_a_recorded_no_lock_installation() {
        use std::os::unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, PermissionsExt},
        };

        use turborepo_tool_install::{Record, Store, Tool};
        let sha = |bytes: &[u8]| format!("{:x}", Sha256::digest(bytes));
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let root = temp.path().join("repo");
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::create_dir_all(root.join("apps/web")).unwrap();
        let root = root.canonicalize().unwrap();
        let web = root.join("apps/web");
        fs::write(
            root.join("turbo.json"),
            r#"{"futureFlags":{"experimentalSetup":true}}"#,
        )
        .unwrap();
        let tool = Tool {
            id: "node".into(),
            version: "24.0.0".into(),
            platform: "test".into(),
            artifact_sha256: "a".repeat(64),
            executables: [("node".into(), "bin/node".into())].into(),
        };
        let selection = br#"{"schemaVersion":0,"tools":{}}"#.to_vec();
        let mut store = Store::open(&root).unwrap();
        let expected = store.generation().unwrap();
        store
            .reconcile_recorded_checked(
                &[tool],
                &Record::new(selection.clone()).unwrap(),
                &expected,
                false,
                |_, tree| {
                    fs::create_dir_all(tree.join("bin"))?;
                    fs::write(tree.join("bin/node"), "#!/bin/sh\n")?;
                    fs::set_permissions(tree.join("bin/node"), fs::Permissions::from_mode(0o755))?;
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
        drop(store);
        assert_eq!(find_root(&web).unwrap(), root);
        // Unlike a turbo.lock root, an unrecorded inventory is not an opt-in.
        let off = changes(None, "/usr/bin", Some(&web), Some(&home));
        assert!(off.path.is_none() && off.notice.is_none());

        let current = Store::inspect(&root).unwrap().unwrap();
        let identity = fs::metadata(&root).unwrap();
        let record = serde_json::json!({"schema": 1, "installation": {
            "root": root, "device": identity.dev(), "inode": identity.ino(),
            "manifest_sha256": sha(&fs::read(root.join(".turbo/tools/manifest.json")).unwrap()),
            "bin": current.bin, "tools": current.tools, "selection_sha256": sha(&selection),
        }});
        let records = home.join("turborepo/setup-roots");
        fs::create_dir_all(&records).unwrap();
        for dir in [home.clone(), home.join("turborepo"), records.clone()] {
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let path = records.join(format!("{}.json", sha(root.as_os_str().as_bytes())));
        fs::write(&path, record.to_string()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let bin = current.bin.to_str().unwrap();
        let active = changes(None, "/usr/bin", Some(&web), Some(&home));
        assert_eq!(active.path, Some(prepend("/usr/bin", bin)));
        assert!(active.notice.is_none());
        // A lock added later is another resolution: tools stay, with a notice.
        fs::write(root.join("turbo.lock"), "{}").unwrap();
        let stale = changes(None, "/usr/bin", Some(&web), Some(&home));
        assert_eq!(stale.path, Some(prepend("/usr/bin", bin)));
        assert!(stale.notice.unwrap().contains("turbo.lock changed"));
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
            &p("/mine:/repo/bin:/usr/bin"),
            Some(temp.path()),
            None,
        );
        assert_eq!(left.path, Some(p("/mine:/usr/bin")));
        assert_eq!(left.state, Some(None));
        assert_eq!(
            changes(None, "/usr/bin", Some(temp.path()), None),
            Changes::default()
        );
    }

    #[test]
    fn only_windows_go_activations_set_runtime_variables() {
        let bin = r"C:\repo\.turbo\tools\generation-a\bin";
        let goroot = r"C:\repo\.turbo\tools\generation-a\tools\go".to_owned();
        assert!(runtime(bin, false).is_empty());
        if !cfg!(windows) {
            assert!(runtime(bin, true).is_empty());
            return;
        }
        let wanted = runtime(bin, true);
        assert_eq!(
            wanted,
            [
                ("GOROOT", goroot.clone()),
                ("GOTOOLCHAIN", "local".to_owned())
            ]
        );
        let s = |value: &str| Some(value.to_owned());
        // Entering saves the shell's values, set or not, then sets both.
        let shell = |name: &str| (name == "GOROOT").then(|| r"C:\Go".to_owned());
        let (edits, saved) = runtime_changes(None, &wanted, &shell);
        assert_eq!(
            edits,
            [
                ("GOROOT".to_owned(), s(&goroot)),
                ("GOTOOLCHAIN".to_owned(), s("local"))
            ]
        );
        assert_eq!(
            saved,
            [
                ("GOROOT".to_owned(), s(r"C:\Go")),
                ("GOTOOLCHAIN".to_owned(), None)
            ]
        );
        let state = State {
            bin: Some(bin.into()),
            saved,
            ..Default::default()
        };
        // An unchanged activation keeps the user's own edit inside the repo,
        // and leaving restores only what still holds the hook's value.
        let edited = |name: &str| {
            s(if name == "GOROOT" {
                r"D:\mine"
            } else {
                "local"
            })
        };
        let (edits, kept) = runtime_changes(Some(&state), &wanted, &edited);
        assert!(edits.is_empty());
        assert_eq!(kept, state.saved);
        let (edits, kept) = runtime_changes(Some(&state), &[], &edited);
        assert_eq!(edits, [("GOTOOLCHAIN".to_owned(), None)]);
        assert!(kept.is_empty());
        let activated = |name: &str| {
            s(if name == "GOROOT" {
                goroot.as_str()
            } else {
                "local"
            })
        };
        let (edits, _) = runtime_changes(Some(&state), &[], &activated);
        assert_eq!(
            edits,
            [
                ("GOROOT".to_owned(), s(r"C:\Go")),
                ("GOTOOLCHAIN".to_owned(), None)
            ]
        );
    }
}
