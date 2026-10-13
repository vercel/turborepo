//! `turbo hook <shell>`: prompt-time activation of the tools `turbo setup`
//! recorded in this user's state directory. Runs in the invoking global binary,
//! never a repository-local turbo, and never resolves, downloads, installs, or
//! writes locks or user state. Out-of-date tools stay active with a notice;
//! `turbo run` stays strict.

use std::{
    collections::BTreeMap,
    fmt::Write as _,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use turbopath::AbsoluteSystemPath;
use turborepo_turbo_json::RawTurboJson;

use crate::{cli::HookShell, commands::setup::trust::TrustStore};

/// Exported so nested shells agree on what the hook added. Internal only.
const STATE: &str = "__TURBO_HOOK_STATE";
/// The variables this hook set and what each replaced, so leaving restores
/// them exactly. Internal only, and absent while the hook sets none.
const ENV_STATE: &str = "__TURBO_HOOK_ENV";
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
    /// A musl host, where managed tools never activate.
    Unsupported,
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
            "unsupported" => Status::Unsupported,
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
            Status::Unsupported => "unsupported",
        };
        let bin = self.bin.as_deref().unwrap_or_default();
        format!("1;{};{};{status};{bin}", self.root, self.stamp)
    }
}

/// Variables to set, or unset when `None`.
type EnvChanges = Vec<(String, Option<String>)>;

#[derive(Debug, Default, PartialEq, Eq)]
struct Changes {
    path: Option<String>,
    /// `Some(None)` unsets the state.
    state: Option<Option<String>>,
    env: EnvChanges,
    /// `Some(None)` unsets the variable state.
    env_state: Option<Option<String>>,
    notice: Option<String>,
}

/// Why a recorded installation stays inactive.
enum Reason {
    /// Setup's own checks; `turbo setup` reports or repairs it.
    Setup(String),
    /// The shell's PATHEXT lacks these export types.
    Pathext(String),
}

/// Host facts read once per prompt.
struct Host {
    /// Setup provisions here. Windows arm64 hosts never activate, even with a
    /// record an older turbo left behind.
    qualified: bool,
    /// Windows shells find exports only through PATHEXT. None elsewhere.
    pathext: Option<String>,
}

impl Host {
    fn current() -> Self {
        Self {
            qualified: !cfg!(windows) || crate::commands::setup::platform_supported(),
            pathext: cfg!(windows).then(|| std::env::var("PATHEXT").unwrap_or_default()),
        }
    }
}

fn name(shell: HookShell) -> &'static str {
    match shell {
        HookShell::Bash => "bash",
        HookShell::Zsh => "zsh",
        HookShell::Fish => "fish",
        HookShell::PowerShell => "powershell",
    }
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Saved {
    set: String,
    previous: Option<String>,
}

/// Apply `recorded` over what this hook set before (`saved`), given the
/// shell's current values. A variable the user changed since is theirs: it is
/// neither restored nor set again.
fn env_changes(
    saved: Option<&str>,
    recorded: &BTreeMap<String, String>,
    current: &dyn Fn(&str) -> Option<String>,
) -> (EnvChanges, Option<Option<String>>) {
    // Only variables the hook can set are restored, so a tampered state cannot
    // unset or overwrite any other variable.
    let saved: BTreeMap<String, Saved> = saved
        .and_then(|saved| serde_json::from_str::<BTreeMap<String, Saved>>(saved).ok())
        .filter(|saved| {
            (saved.keys())
                .all(|name| turborepo_setup::activation::HOOK_ENV.contains(&name.as_str()))
        })
        .unwrap_or_default();
    let mut changes = Vec::new();
    let mut next = BTreeMap::new();
    let names: std::collections::BTreeSet<_> = saved.keys().chain(recorded.keys()).collect();
    for name in names {
        let now = current(name);
        let underlying = match saved.get(name) {
            Some(old) if now.as_deref() != Some(old.set.as_str()) => continue,
            Some(old) => old.previous.clone(),
            None => now.clone(),
        };
        let target = match recorded.get(name) {
            Some(value) => {
                let previous = underlying;
                next.insert(
                    name.clone(),
                    Saved {
                        set: value.clone(),
                        previous,
                    },
                );
                Some(value.clone())
            }
            None => underlying,
        };
        if target != now {
            changes.push((name.clone(), target));
        }
    }
    let state = match (next == saved, next.is_empty()) {
        (true, _) => None,
        (false, true) => Some(None),
        (false, false) => serde_json::to_string(&next).ok().map(Some),
    };
    (changes, state)
}

pub(crate) fn run(shell: HookShell, export: bool) -> i32 {
    // These shells keep a POSIX PATH that this native binary would rewrite in
    // Windows form, so on native Windows only PowerShell is supported.
    if cfg!(windows) && shell != HookShell::PowerShell {
        if export {
            return 0;
        }
        let _ = writeln!(
            std::io::stderr(),
            "turbo: `turbo hook {}` is not supported on native Windows. Use `turbo hook \
             powershell` in PowerShell, or `turbo exec -- <command>` in other shells.",
            name(shell)
        );
        return 1;
    }
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
        let host = Host::current();
        let musl = || {
            matches!(
                turborepo_platform::host_libc(),
                Ok(Some(turborepo_platform::Libc::Musl { .. }))
            )
        };
        let changes = changes(
            previous,
            &path,
            cwd.as_deref(),
            base.as_deref(),
            &host,
            musl,
        );
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
    host: &Host,
    musl: impl FnOnce() -> bool,
) -> Changes {
    changes_with(previous, path, cwd, base, host, musl, &|name| {
        std::env::var(name).ok()
    })
}

/// `current` reads the shell's environment.
fn changes_with(
    previous: Option<State>,
    path: &str,
    cwd: Option<&Path>,
    base: Option<&Path>,
    host: &Host,
    musl: impl FnOnce() -> bool,
    current: &dyn Fn(&str) -> Option<String>,
) -> Changes {
    let saved = current(ENV_STATE);
    let Some(root) = cwd.and_then(find_root) else {
        // Outside every repository: remove only what the hook added.
        let mut left = previous.map_or_else(Changes::default, |previous| Changes {
            path: previous.bin.map(|bin| remove(path, &bin)),
            state: Some(None),
            ..Default::default()
        });
        (left.env, left.env_state) = env_changes(saved.as_deref(), &BTreeMap::new(), current);
        return left;
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
    if let Some(pathext) = &host.pathext {
        stamp.extend(pathext.as_bytes());
    }
    let same_root = previous.as_ref().filter(|previous| previous.root == id);
    if same_root.is_some_and(|same| same.stamp == finish(&stamp, same.bin.as_deref())) {
        return Changes::default();
    }

    // Where setup refuses to provision (Windows arm64), `turbo setup` cannot
    // help: stay off rather than print a notice. Without turbo.lock only this
    // user's `turbo setup --no-lock` record opts a clone in, so an unrecorded
    // lockless clone stays off quietly too.
    let locked = fs::symlink_metadata(root.join("turbo.lock")).is_ok();
    let mut recorded_env = BTreeMap::new();
    let (status, bin, reason) = if !cfg!(any(unix, windows))
        || !host.qualified
        || !enabled(&root)
        || (locked && external(&root))
    {
        // Like `turbo run`, verify-system tools stay on the caller's PATH and
        // need no setup record: there is nothing to activate.
        (Status::Off, None, None)
    } else if musl() {
        // Like `turbo run`, musl hosts never activate managed artifacts.
        let status = if locked {
            Status::Unsupported
        } else {
            Status::Off
        };
        (status, None, None)
    } else {
        match trust.and_then(|trust| trust.recorded().map_err(|e| e.to_string())) {
            Ok(Some(recorded)) => {
                // Like the PATH entry, recorded variables must be exact strings.
                let env: Option<BTreeMap<_, _>> = recorded
                    .env
                    .into_iter()
                    .map(|(name, value)| value.into_string().ok().map(|value| (name, value)))
                    .collect();
                match (path_entry(recorded.bin), env) {
                    (Some(bin), Some(env)) => match host
                        .pathext
                        .as_deref()
                        .and_then(|pathext| unresolvable(&recorded.targets, pathext))
                    {
                        Some(types) => (Status::Missing, None, Some(Reason::Pathext(types))),
                        None => {
                            recorded_env = env;
                            let status = if recorded.stale {
                                Status::Stale
                            } else {
                                Status::Active
                            };
                            (status, Some(bin), None)
                        }
                    },
                    _ => (
                        Status::Missing,
                        None,
                        Some(Reason::Setup("its path cannot be added to PATH".to_owned())),
                    ),
                }
            }
            _ if !locked => (Status::Off, None, None),
            Ok(None) => (Status::Missing, None, None),
            Err(reason) => (Status::Missing, None, Some(Reason::Setup(reason))),
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
        .then(|| notice(status, reason.as_ref()))
        .flatten();
    let state = State {
        root: id,
        stamp: finish(&stamp, bin.as_deref()),
        status,
        bin,
    };
    let (env, env_state) = env_changes(saved.as_deref(), &recorded_env, current);
    Changes {
        path,
        state: Some(Some(state.encode())),
        env,
        env_state,
        notice,
    }
}

fn notice(status: Status, reason: Option<&Reason>) -> Option<String> {
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
        (Status::Missing, Some(Reason::Setup(reason))) => Some(format!(
            "cannot activate this repository's managed tools ({reason}). Run `turbo setup`."
        )),
        (Status::Missing, Some(Reason::Pathext(types))) => Some(format!(
            "this repository's managed tools need {types} in PATHEXT, so they are not active."
        )),
        (Status::Unsupported, _) => Some(
            "managed tools are not supported on musl Linux hosts such as Alpine, so none are \
             active. Install the repository's tools in the image and use verify-system for them."
                .to_owned(),
        ),
        (Status::Off | Status::Active, _) => None,
    }
}

/// turbo.lock selects only verify-system tools. An unreadable lock is not.
fn external(root: &Path) -> bool {
    turborepo_setup::lock::Lock::read(root)
        .ok()
        .flatten()
        .is_some_and(|lock| !lock.tools().is_empty() && lock.verify_system_only())
}

/// The recorded bin as a PATH entry. Windows shells and `.cmd` forwarders
/// need the ordinary spelling setup records, never a verbatim `\\?\` path.
fn path_entry(bin: PathBuf) -> Option<String> {
    use std::path::{Component, Prefix};
    let verbatim = matches!(
        bin.components().next(),
        Some(Component::Prefix(prefix)) if matches!(
            prefix.kind(),
            Prefix::Verbatim(_) | Prefix::VerbatimUNC(..) | Prefix::VerbatimDisk(_)
                | Prefix::DeviceNS(_)
        )
    );
    let bin = bin.into_os_string().into_string().ok()?;
    (!verbatim && !bin.contains(SEPARATOR)).then_some(bin)
}

/// The export types PATHEXT cannot resolve by bare name, if any. Windows
/// exports are `.exe` hard links and `.cmd` forwarders. PowerShell, like
/// cmd.exe, tries each PATHEXT entry in every PATH directory before the next,
/// so a prepended bin wins once its types are listed. PowerShell splits on `;`
/// without trimming and resolves nothing by bare name when PATHEXT is unset.
fn unresolvable(targets: &[String], pathext: &str) -> Option<String> {
    let listed: Vec<_> = pathext.split(';').filter(|e| !e.is_empty()).collect();
    let mut missing: Vec<String> = Vec::new();
    for target in targets {
        let Some(extension) = Path::new(target).extension().and_then(|e| e.to_str()) else {
            continue;
        };
        let extension = format!(".{}", extension.to_ascii_uppercase());
        if !listed.iter().any(|e| e.eq_ignore_ascii_case(&extension))
            && !missing.contains(&extension)
        {
            missing.push(extension);
        }
    }
    missing.sort();
    (!missing.is_empty()).then(|| missing.join(" and "))
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

/// A lock activates tools only beside a valid root turbo.json or turbo.jsonc,
/// so a stray turbo.lock outside a Turborepo repository stays quiet.
fn enabled(root: &Path) -> bool {
    let Ok(root) = AbsoluteSystemPath::from_std_path(root) else {
        return false;
    };
    turborepo_config::resolve_turbo_config_path(root)
        .ok()
        .and_then(|config| RawTurboJson::read(root, &config, true).ok().flatten())
        .is_some()
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
    // NTFS file IDs need an open handle; times and size are enough for this
    // cache key because setup writes a fresh file for every change.
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        for value in [
            u64::from(metadata.file_attributes()),
            metadata.creation_time(),
            metadata.last_write_time(),
            metadata.file_size(),
        ] {
            stamp.extend(value.to_le_bytes());
        }
    }
    #[cfg(not(any(unix, windows)))]
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
    let mut apply = |name: &str, value: Option<&str>| {
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
        apply("PATH", Some(path));
    }
    for (name, value) in &changes.env {
        apply(name, value.as_deref());
    }
    if let Some(state) = &changes.env_state {
        apply(ENV_STATE, state.as_deref());
    }
    if let Some(state) = &changes.state {
        apply(STATE, state.as_deref());
    }
    out
}

/// One prompt's PowerShell export, the state it leaves, and the variable state
/// it leaves (`__TURBO_HOOK_ENV`), for setup's native Windows fixtures. `vars`
/// is the shell's environment, including the hook's own variable state.
#[cfg(all(test, windows))]
pub(crate) fn powershell_for_tests(
    state: Option<&str>,
    path: &str,
    cwd: &Path,
    base: &Path,
    vars: impl Fn(&str) -> Option<String>,
) -> (String, Option<String>, Option<String>) {
    let previous = state.and_then(State::parse);
    let changes = changes_with(
        previous,
        path,
        Some(cwd),
        Some(base),
        &Host::current(),
        || false,
        &vars,
    );
    (
        render(HookShell::PowerShell, &changes),
        changes.state.flatten(),
        changes.env_state.flatten(),
    )
}

/// The PATH and variables the hook applies on entering `cwd`, for native
/// Windows setup journeys.
#[cfg(all(test, windows))]
pub(crate) fn entering_for_tests(
    cwd: &Path,
    base: &Path,
    path: &str,
) -> (Option<String>, EnvChanges) {
    let changes = changes_with(
        None,
        path,
        Some(cwd),
        Some(base),
        &Host::current(),
        || false,
        &|_| None,
    );
    (changes.path, changes.env)
}

fn quote(shell: HookShell, value: &str) -> String {
    if shell == HookShell::PowerShell {
        return powershell(value);
    }
    let mut quoted = String::from("'");
    for c in value.chars() {
        match (shell, c) {
            (HookShell::Bash | HookShell::Zsh, '\'') => quoted.push_str(r"'\''"),
            (HookShell::Fish, '\'' | '\\') => {
                quoted.push('\\');
                quoted.push(c);
            }
            _ => quoted.push(c),
        }
    }
    quoted.push('\'');
    quoted
}

/// PowerShell decodes a native command's output with
/// `[Console]::OutputEncoding`, an OEM code page on most Windows consoles, so
/// UTF-8 paths would come back corrupted. Print only printable ASCII: every
/// other character becomes `[char]` UTF-16 units appended to single-quoted
/// literals, which Windows PowerShell 5.1 and PowerShell 7 both evaluate. That
/// also keeps the typographic quotes PowerShell treats as delimiters out of the
/// literals.
fn powershell(value: &str) -> String {
    let mut parts = Vec::new();
    let mut literal = String::from("'");
    for c in value.chars() {
        match c {
            '\'' => literal.push_str("''"),
            ' '..='~' => literal.push(c),
            _ => {
                // The first operand stays a string, so `+` concatenates.
                if literal.len() > 1 || parts.is_empty() {
                    parts.push(std::mem::take(&mut literal) + "'");
                }
                literal = String::from("'");
                let mut units = [0; 2];
                for unit in c.encode_utf16(&mut units) {
                    parts.push(format!("[char]0x{unit:04X}"));
                }
            }
        }
    }
    if parts.is_empty() {
        return literal + "'";
    }
    if literal.len() > 1 {
        parts.push(literal + "'");
    }
    format!("({})", parts.join(" + "))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    /// Writes test PATHs with `:` and converts them to the native separator.
    fn p(path: &str) -> String {
        path.replace(':', &SEPARATOR.to_string())
    }

    /// A qualified host without PATHEXT, like every Unix host, in a shell
    /// that has none of the runtime variables set.
    fn changes(
        previous: Option<State>,
        path: &str,
        cwd: Option<&Path>,
        base: Option<&Path>,
        musl: impl FnOnce() -> bool,
    ) -> Changes {
        let host = Host {
            qualified: true,
            pathext: None,
        };
        super::changes_with(previous, path, cwd, base, &host, musl, &|_| None)
    }

    #[test]
    fn path_entries_use_the_plain_spelling() {
        assert_eq!(path_entry("/repo/bin".into()).as_deref(), Some("/repo/bin"));
        assert_eq!(path_entry(p("/a:/b").into()), None);
        if cfg!(windows) {
            assert_eq!(
                path_entry(r"C:\repo\bin".into()).as_deref(),
                Some(r"C:\repo\bin")
            );
            for verbatim in [
                r"\\?\C:\repo\bin",
                r"\\?\UNC\server\share\bin",
                r"\\.\C:\bin",
            ] {
                assert_eq!(path_entry(verbatim.into()), None, "{verbatim}");
            }
        }
    }

    #[test]
    fn pathext_must_list_every_export_type() {
        let targets = ["node.exe", "npm.cmd", "launch/pnpm.cmd"].map(String::from);
        for listed in [".COM;.EXE;.BAT;.CMD", ".exe;.cmd", ";.Cmd;;.EXE"] {
            assert_eq!(unresolvable(&targets, listed), None, "{listed}");
        }
        assert_eq!(unresolvable(&targets, ".COM;.EXE").as_deref(), Some(".CMD"));
        // PowerShell neither trims entries nor defaults an unset PATHEXT.
        assert_eq!(
            unresolvable(&targets, " .CMD; .EXE").as_deref(),
            Some(".CMD and .EXE")
        );
        assert_eq!(unresolvable(&targets, "").as_deref(), Some(".CMD and .EXE"));
        assert_eq!(unresolvable(&[], ""), None);
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
        // Console code pages cannot corrupt PowerShell output: it is ASCII,
        // and typographic quotes never appear inside a literal.
        assert_eq!(
            quote(HookShell::PowerShell, "a\u{2019}b"),
            "('a' + [char]0x2019 + 'b')"
        );
        assert_eq!(
            quote(HookShell::PowerShell, "\u{fc}\u{1F600}\t'"),
            "('' + [char]0x00FC + [char]0xD83D + [char]0xDE00 + [char]0x0009 + '''')"
        );
        assert_eq!(quote(HookShell::PowerShell, ""), "''");
        let unicode = Changes {
            path: Some("C:\\J\u{fc}rgen\\bin;C:\\Windows".into()),
            state: Some(Some("1;a;b;active;C:\\J\u{fc}rgen\\bin".into())),
            ..Default::default()
        };
        assert!(render(HookShell::PowerShell, &unicode).is_ascii());
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
    fn variables_are_set_then_restored_exactly_and_user_edits_win() {
        use std::{cell::RefCell, collections::HashMap};
        let owned = |pairs: &[(&str, &str)]| -> Vec<(String, String)> {
            pairs
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect()
        };
        let before: HashMap<_, _> = owned(&[("CARGO_HOME", r"C:\Users\me\.cargo")])
            .into_iter()
            .collect();
        let shell = RefCell::new(before.clone());
        let recorded: BTreeMap<_, _> =
            owned(&[("CARGO_HOME", r"C:\repo\cargo"), ("RUSTUP_HOME", r"C:\r")])
                .into_iter()
                .collect();
        // Apply one round of changes to the shell, as `eval` would.
        let step = |recorded: &BTreeMap<String, String>| {
            let current = |name: &str| shell.borrow().get(name).cloned();
            let (env, state) = env_changes(current(ENV_STATE).as_deref(), recorded, &current);
            let mut shell = shell.borrow_mut();
            let state = state.map(|state| (ENV_STATE.to_owned(), state));
            for (name, value) in env.iter().cloned().chain(state) {
                match value {
                    Some(value) => shell.insert(name, value),
                    None => shell.remove(&name),
                };
            }
            env
        };
        assert_eq!(step(&recorded).len(), 2);
        assert_eq!(shell.borrow()["CARGO_HOME"], r"C:\repo\cargo");
        // Unchanged values need no output.
        assert!(step(&recorded).is_empty());
        // Leaving restores the previous value and removes what was unset.
        step(&BTreeMap::new());
        assert_eq!(*shell.borrow(), before);
        // A variable the user changed inside stays theirs on leaving.
        step(&recorded);
        shell
            .borrow_mut()
            .insert("RUSTUP_HOME".into(), r"D:\mine".into());
        step(&BTreeMap::new());
        assert_eq!(shell.borrow()["RUSTUP_HOME"], r"D:\mine");
        assert_eq!(shell.borrow()["CARGO_HOME"], r"C:\Users\me\.cargo");
        assert!(!shell.borrow().contains_key(ENV_STATE));
        // Without recorded or saved variables there is nothing to do.
        let none = |_: &str| None;
        assert_eq!(env_changes(None, &BTreeMap::new(), &none), (vec![], None));
        // A state naming any variable the hook never sets is ignored whole.
        let tampered =
            r#"{"PATH":{"set":"x","previous":null},"GOROOT":{"set":"y","previous":null}}"#;
        let both = |name: &str| (name == "PATH" || name == "GOROOT").then(|| "x".to_owned());
        assert_eq!(
            env_changes(Some(tampered), &BTreeMap::new(), &both),
            (vec![], None)
        );
    }

    #[test]
    fn only_windows_go_and_rust_activations_export_runtime_variables() {
        use turborepo_setup::activation::{HOOK_ENV, hook_env};
        let tool = |id: &str| turborepo_tool_install::Tool {
            id: id.into(),
            version: "1.90.0".into(),
            platform: "windows-x64".into(),
            artifact_sha256: "a".repeat(64),
            executables: BTreeMap::new(),
        };
        let root = Path::new(r"C:\repo");
        let bin = root.join(r".turbo\tools\generation-a\bin");
        assert!(hook_env(root, &bin, &[tool("node"), tool("pnpm")]).is_empty());
        let env = hook_env(root, &bin, &[tool("node"), tool("go"), tool("rust")]);
        if !cfg!(windows) {
            assert!(env.is_empty());
            return;
        }
        let names: Vec<_> = env.keys().map(String::as_str).collect();
        assert_eq!(names, HOOK_ENV);
        let tools = root.join(r".turbo\tools\generation-a\tools");
        assert_eq!(env["GOROOT"], tools.join("go").into_os_string());
        assert_eq!(env["GOTOOLCHAIN"], "local");
        assert_eq!(env["RUSTUP_HOME"], tools.join("rust").into_os_string());
        assert_eq!(env["RUSTUP_TOOLCHAIN"], "1.90.0-x86_64-pc-windows-msvc");
        assert_eq!(env["RUSTUP_AUTO_INSTALL"], "0");
    }

    #[test]
    fn variables_render_and_unset_in_every_shell() {
        let changes = Changes {
            env: vec![
                ("RUSTUP_HOME".into(), Some("C:\\r".into())),
                ("X".into(), None),
            ],
            env_state: Some(None),
            ..Default::default()
        };
        assert_eq!(
            render(HookShell::PowerShell, &changes),
            "$env:RUSTUP_HOME = 'C:\\r'\nRemove-Item -LiteralPath Env:X -ErrorAction \
             SilentlyContinue\nRemove-Item -LiteralPath Env:__TURBO_HOOK_ENV -ErrorAction \
             SilentlyContinue\n"
        );
        assert_eq!(
            render(HookShell::Bash, &changes),
            "export RUSTUP_HOME='C:\\r';\nunset X;\nunset __TURBO_HOOK_ENV;\n"
        );
        assert_eq!(
            render(HookShell::Fish, &changes),
            "set -gx RUSTUP_HOME 'C:\\\\r';\nset -e X;\nset -e __TURBO_HOOK_ENV;\n"
        );
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
        let off = changes(None, "/usr/bin", Some(&outer), Some(&home), || false);
        assert!(off.path.is_none() && off.notice.is_none());
        let off_state = State::parse(off.state.clone().unwrap().as_deref().unwrap());
        // Unchanged inputs take the fast path: no output at all.
        assert_eq!(
            changes(off_state, "/usr/bin", Some(&outer), Some(&home), || false),
            Changes::default()
        );

        fs::write(outer.join("turbo.json"), "{}").unwrap();
        let missing = changes(None, "/usr/bin", Some(&outer), Some(&home), || false);
        assert!(missing.path.is_none());
        assert!(missing.notice.unwrap().contains("not installed"));
        let state = State::parse(missing.state.unwrap().as_deref().unwrap());
        assert_eq!(state.as_ref().unwrap().status, Status::Missing);
        assert_eq!(
            changes(state, "/usr/bin", Some(&outer), Some(&home), || false),
            Changes::default()
        );
    }

    #[cfg(unix)]
    #[test]
    fn verify_system_locks_and_musl_hosts_never_activate_managed_tools() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let root = temp.path().join("repo");
        fs::create_dir_all(root.join(".git")).unwrap();
        let root = root.canonicalize().unwrap();
        fs::write(
            root.join("turbo.json"),
            r#"{"futureFlags":{"experimentalSetup":true}}"#,
        )
        .unwrap();
        let node = |kind: &str| {
            let installation = match kind {
                "verify-system" => r#"{"kind":"verify-system","executables":["node"]}"#,
                _ => r#"{"kind":"managed","artifacts":{}}"#,
            };
            format!(
                r#"{{"schemaVersion":0,"tools":{{"node":{{"adapter":"node","version":"24.0.0",
                "declarations":[{{"file":".nvmrc","request":"24.0.0"}}],"installation":{installation}}}}}}}"#
            )
        };
        let status = |changes: &Changes| {
            State::parse(changes.state.clone().unwrap().as_deref().unwrap())
                .unwrap()
                .status
        };
        // verify-system tools need no setup record and stay on the caller's
        // PATH, so no host is told to run `turbo setup`.
        fs::write(root.join("turbo.lock"), node("verify-system")).unwrap();
        for musl in [false, true] {
            let off = changes(None, "/usr/bin", Some(&root), Some(&home), || musl);
            assert!(off.path.is_none() && off.notice.is_none(), "{off:?}");
            assert_eq!(status(&off), Status::Off);
        }
        // A lock that needs managed tools says why musl hosts activate none.
        fs::write(root.join("turbo.lock"), node("managed")).unwrap();
        let musl = changes(None, "/usr/bin", Some(&root), Some(&home), || true);
        assert!(musl.path.is_none(), "{musl:?}");
        assert!(musl.notice.as_deref().unwrap().contains("musl"), "{musl:?}");
        assert_eq!(status(&musl), Status::Unsupported);
        let glibc = changes(None, "/usr/bin", Some(&root), Some(&home), || false);
        assert!(glibc.notice.unwrap().contains("not installed"));
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
        let off = changes(None, "/usr/bin", Some(&web), Some(&home), || false);
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
        let active = changes(None, "/usr/bin", Some(&web), Some(&home), || false);
        assert_eq!(active.path, Some(prepend("/usr/bin", bin)));
        assert!(active.notice.is_none());
        // A host setup refuses, like Windows arm64, activates nothing, even with
        // a record an older turbo left there.
        let refused = Host {
            qualified: false,
            pathext: None,
        };
        let off = changes_with(
            None,
            "/usr/bin",
            Some(&web),
            Some(&home),
            &refused,
            || false,
            &|_| None,
        );
        assert!(off.path.is_none() && off.notice.is_none());
        assert!(off.state.unwrap().unwrap().contains(";off;"));
        // A lock added later is another resolution: tools stay, with a notice.
        fs::write(root.join("turbo.lock"), "{}").unwrap();
        let stale = changes(None, "/usr/bin", Some(&web), Some(&home), || false);
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
            || false,
        );
        assert_eq!(left.path, Some(p("/mine:/usr/bin")));
        assert_eq!(left.state, Some(None));
        assert_eq!(
            changes(None, "/usr/bin", Some(temp.path()), None, || false),
            Changes::default()
        );
    }
}
