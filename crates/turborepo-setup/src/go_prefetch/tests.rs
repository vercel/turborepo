#![allow(clippy::unwrap_used)]

use super::*;

// A stand-in for the managed toolchain: it logs its environment and arguments,
// then follows the scenario written next to it.
#[cfg(unix)]
const FAKE_GO: (&str, &str) = (
    "bin/go",
    r#"#!/bin/sh
dir=$(dirname "$0")
proxy=inherited
[ "$GOPROXY" = off ] && proxy=off
echo "GOTOOLCHAIN=$GOTOOLCHAIN GOPROXY=$proxy $*" >> "$dir/log"
if [ "$1" = list ]; then
  echo "$PWD" > "$dir/cwd"
  echo "$PWD/go.mod"
  echo "$PWD/member/go.mod"
  exit 0
fi
case $(cat "$dir/scenario") in
  ok) exit 0 ;;
  mutate) echo "example.com/x v1.0.0/go.mod h1:new=" >> member/go.sum; touch go.work.sum ;;
  missing) echo "go: example.com/x@v1.0.0: missing go.sum entry for go.mod file" >&2; exit 1 ;;
  offline) echo "go: example.com/x@v1.0.0: module lookup disabled by GOPROXY=off" >&2; exit 1 ;;
  fail) echo "boom" >&2; exit 7 ;;
esac
"#,
);
// Native Windows runs `.cmd` files through cmd.exe, which receives the
// arguments Rust quoted for it; dropping the quotes logs the same line.
#[cfg(windows)]
const FAKE_GO: (&str, &str) = (
    "bin/go.cmd",
    r#"@echo off
set "proxy=inherited"
if "%GOPROXY%"=="off" set "proxy=off"
set "args=%*"
set "args=%args:"=%"
>>"%~dp0log" echo GOTOOLCHAIN=%GOTOOLCHAIN% GOPROXY=%proxy% %args%
if "%~1"=="list" (
  >"%~dp0cwd" echo %CD%
  echo %CD%\go.mod
  echo %CD%\member\go.mod
  exit /b 0
)
set /p scenario=<"%~dp0scenario"
if "%scenario%"=="ok" exit /b 0
if "%scenario%"=="mutate" (
  >>member\go.sum echo example.com/x v1.0.0/go.mod h1:new=
  type nul >go.work.sum
  exit /b 0
)
if "%scenario%"=="missing" (
  >&2 echo go: example.com/x@v1.0.0: missing go.sum entry for go.mod file
  exit /b 1
)
if "%scenario%"=="offline" (
  >&2 echo go: example.com/x@v1.0.0: module lookup disabled by GOPROXY=off
  exit /b 1
)
>&2 echo boom
exit /b 7
"#,
);

struct Fixture {
    temp: tempfile::TempDir,
}
impl Fixture {
    fn new(scenario: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        fs::create_dir_all(root.join("member")).unwrap();
        fs::create_dir_all(temp.path().join("bin")).unwrap();
        for (path, contents) in [
            ("repo/go.work", "go 1.22\nuse (\n\t.\n\t./member\n)\n"),
            ("repo/go.mod", "module example.com/root\ngo 1.22\n"),
            ("repo/go.sum", "example.com/y v1.0.0 h1:committed=\n"),
            ("repo/member/go.mod", "module example.com/member\ngo 1.22\n"),
            ("repo/member/go.sum", "example.com/x v1.0.0 h1:committed=\n"),
            FAKE_GO,
            ("bin/scenario", scenario),
        ] {
            // cmd.exe reads batch files with CRLF line endings.
            let contents = match path.ends_with(".cmd") {
                true => contents.replace('\n', "\r\n"),
                false => contents.to_owned(),
            };
            fs::write(temp.path().join(path), contents).unwrap();
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let go = temp.path().join(FAKE_GO.0);
            fs::set_permissions(&go, fs::Permissions::from_mode(0o755)).unwrap();
        }
        Self { temp }
    }
    fn run(&self, offline: bool) -> Result<(), Error> {
        Prefetch {
            go: &self.temp.path().join(FAKE_GO.0),
            root: &self.temp.path().join("repo"),
            offline,
            env: &BTreeMap::new(),
        }
        .run()
    }
    fn read(&self, file: &str) -> String {
        let text = fs::read_to_string(self.temp.path().join("bin").join(file)).unwrap();
        text.replace("\r\n", "\n")
    }
    fn committed(&self) -> Vec<Option<Vec<u8>>> {
        [
            "go.work",
            "go.work.sum",
            "go.mod",
            "go.sum",
            "member/go.mod",
            "member/go.sum",
        ]
        .map(|file| read(&self.temp.path().join("repo").join(file)).unwrap())
        .to_vec()
    }
}

#[test]
fn prefetch_downloads_modules_with_the_locked_toolchain_only() {
    let f = Fixture::new("ok");
    let before = f.committed();
    f.run(false).unwrap();
    assert_eq!(
        f.read("log"),
        "GOTOOLCHAIN=local GOPROXY=inherited list -mod=readonly -m -f \
         {{.GoMod}}\nGOTOOLCHAIN=local GOPROXY=inherited mod download\n"
    );
    // go runs in the ordinary canonical root, never a Windows `\\?\` path.
    let root = canonical(&f.temp.path().join("repo")).unwrap();
    assert_eq!(f.read("cwd").trim_end(), root.to_str().unwrap());
    assert_eq!(f.committed(), before);
}

#[test]
fn checksum_mutations_are_restored_and_reported() {
    let f = Fixture::new("mutate");
    let before = f.committed();
    let error = f.run(false).unwrap_err().to_string();
    let member = Path::new("member").join("go.sum");
    let expected = format!(
        "go mod download changed go.work.sum, {}; restored",
        member.display()
    );
    assert!(error.contains(&expected), "{error}");
    assert_eq!(f.committed(), before);
    let missing = Fixture::new("missing").run(false).unwrap_err();
    assert!(matches!(missing, Error::MissingSums(_)), "{missing}");
}

#[test]
fn offline_and_command_failures_are_classified_with_exit_status() {
    let f = Fixture::new("offline");
    let offline = f.run(true).unwrap_err();
    assert!(
        matches!(&offline, Error::Offline { code: 1, stderr } if stderr.ends_with("disabled by GOPROXY=off")),
        "{offline}"
    );
    assert!(
        f.read("log")
            .ends_with("GOTOOLCHAIN=local GOPROXY=off mod download\n")
    );
    let failed = Fixture::new("fail").run(false).unwrap_err();
    assert!(
        matches!(&failed, Error::Failed { command: "mod download", code: 7, stderr } if stderr == "boom"),
        "{failed}"
    );
}
