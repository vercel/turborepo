# Release smoke for the packaged native Windows standalone turbo. It installs
# the archive with install.ps1 from staged release assets while Node.js, JS
# package managers, Rust, uv, Python, and Go are absent from PATH, and the
# installer adds its PowerShell hook. On x64 it then runs the managed setup
# journey against the loopback fixture mirror (setup-fixture-mirror.mjs) in one
# repository with Node.js and pnpm, Rust, uv and Python, Go, and an additional
# tool, like standalone-setup-journey.sh on Linux and macOS:
#
# - tampered mirror bytes are refused, and nothing is installed;
# - `turbo setup` from a subdirectory provisions every tool, installs locked
#   dependencies with each native frozen installer, and writes an owner-only
#   trusted record;
# - plain `turbo run` uses the managed tools for turbo's own probes and for
#   every task, in strict and loose modes;
# - repeats, CI, offline setup, and `--check` make no requests;
# - the installer's PowerShell hook activates the tools inside the repository
#   and restores PATH exactly outside it;
# - `turbo exec` runs commands with the managed tools (arguments, directory,
#   stdin, exit codes, `.cmd` launchers) and never installs;
# - setup repairs a damaged installation that run and exec refuse.
#
# On an arm64 host, the installer and setup must refuse instead. turbo runs
# with a cleared environment and an unreachable proxy; only the mirror answers.
#
# Usage: scripts/standalone-windows-smoke.ps1 -Version <version> -Assets <standalone-artifacts>
# The assets directory holds the packaged archives and their SHA256SUMS.
# The fixture mirror runs on the runner's Node.js, outside turbo's PATH.
param(
  [Parameter(Mandatory = $true)][string]$Version,
  [Parameter(Mandatory = $true)][string]$Assets
)
$ErrorActionPreference = 'Stop'
$installerPath = Join-Path $PSScriptRoot '..\apps\docs\public\install.ps1'
$archiveName = "turbo-$Version-x86_64-pc-windows-msvc.tar.gz"
$triple = 'x86_64-pc-windows-msvc'
$root = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [IO.Path]::GetTempPath() }
$work = Join-Path $root ('turbo-windows-smoke-' + [Guid]::NewGuid().ToString('N'))
$profilePath = Join-Path $work 'profile\profile.ps1'
$previousPath = $env:Path
$script:Requests = 0
$mirror = $null

function Fail([string]$Message) { throw "standalone Windows smoke: $Message" }

# Miette may wrap diagnostics across lines with `|` gutters.
function Test-Contains([string]$Text, [string]$Expected) {
  $squash = { param($value) $value -replace '[\s|\u2502]', '' }
  return (& $squash $Text).Contains((& $squash $Expected))
}

# The installer downloads only through this function; it serves staged assets.
function Invoke-WebRequest {
  [CmdletBinding()]
  param([string]$Uri, [string]$OutFile, [switch]$UseBasicParsing, [hashtable]$Headers)
  $script:Requests++
  $name = $Uri.Substring($Uri.LastIndexOf('/') + 1)
  $source = Join-Path $script:Served $name
  if ($Uri -ne "https://github.com/vercel/turborepo/releases/download/v$Version/$name" -or -not $OutFile -or -not (Test-Path -LiteralPath $source)) {
    throw "unexpected download: $Uri"
  }
  Copy-Item -LiteralPath $source -Destination $OutFile
}

# With -Hook the installer changes PATH and adds the shell hook as it does for
# users, but writes the hook to $profilePath instead of the runner's profile,
# skips GitHub Actions' PATH file, and the user PATH is restored afterwards.
function Install-Turbo([string]$Directory, [switch]$Hook) {
  $script:Served = $Directory
  if (-not $Hook) {
    return (. $installerPath -Version $Version -InstallDirectory $install -NoModifyPath 6>&1 | ForEach-Object { [string]$_ }) -join "`n"
  }
  $PROFILE = [pscustomobject]@{ CurrentUserAllHosts = $profilePath }
  $githubPath = $env:GITHUB_PATH
  $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
  try {
    $env:GITHUB_PATH = $null
    return (. $installerPath -Version $Version -InstallDirectory $install 6>&1 | ForEach-Object { [string]$_ }) -join "`n"
  } finally {
    $env:GITHUB_PATH = $githubPath
    [Environment]::SetEnvironmentVariable('Path', $userPath, 'User')
  }
}

# CommandLineToArgvW quoting, so arguments reach turbo exactly.
function ConvertTo-Argument([string]$Value) {
  if ($Value -ne '' -and $Value -notmatch '[\s"]') { return $Value }
  return '"' + (($Value -replace '(\\*)"', '$1$1\"') -replace '(\\+)$', '$1$1') + '"'
}

# Runs a program with only $Variables, like `env -i`. Sets Code and Text.
function Start-Captured([string]$Program, [string]$Arguments, [string]$Directory, [hashtable[]]$Variables, [string]$Stdin) {
  $info = New-Object Diagnostics.ProcessStartInfo
  $info.FileName = $Program
  $info.Arguments = $Arguments
  $info.WorkingDirectory = $Directory
  $info.UseShellExecute = $false
  $info.RedirectStandardInput = $true
  $info.RedirectStandardOutput = $true
  $info.RedirectStandardError = $true
  $info.EnvironmentVariables.Clear()
  foreach ($set in $Variables) {
    foreach ($key in $set.Keys) { $info.EnvironmentVariables[$key] = $set[$key] }
  }
  $process = [Diagnostics.Process]::Start($info)
  if ($Stdin) { $process.StandardInput.Write($Stdin) }
  $process.StandardInput.Close()
  $stdout = $process.StandardOutput.ReadToEndAsync()
  $stderr = $process.StandardError.ReadToEndAsync()
  $process.WaitForExit()
  return [pscustomobject]@{ Code = $process.ExitCode; Text = $stdout.Result + $stderr.Result }
}

# Runs turbo with only $environment and $Extra. Sets Code, Text, Requests.
function Invoke-Turbo([string]$Directory, [string[]]$Arguments, [hashtable]$Extra = @{}, [string]$Stdin = $null) {
  $before = @(Get-Content -LiteralPath $requestLog).Count
  $line = ($Arguments | ForEach-Object { ConvertTo-Argument $_ }) -join ' '
  $result = Start-Captured $turbo $line $Directory @($environment, $Extra) $Stdin
  $result | Add-Member -NotePropertyName Requests -NotePropertyValue (@(Get-Content -LiteralPath $requestLog).Count - $before)
  return $result
}

function Assert-Succeeds([string]$Directory, [string[]]$Arguments, [hashtable]$Extra = @{}, [string]$Stdin = $null, [switch]$Offline) {
  $result = Invoke-Turbo $Directory $Arguments $Extra $Stdin
  if ($result.Code -ne 0) { Fail "turbo $Arguments exited $($result.Code): $($result.Text)" }
  if ($Offline -and $result.Requests -ne 0) { Fail "turbo $Arguments made $($result.Requests) mirror requests: $($result.Text)" }
  return $result
}

function Assert-Unchanged([string]$Step) {
  foreach ($file in @('turbo.lock', '.turbo\tools\manifest.json')) {
    if ((Get-FileHash -LiteralPath (Join-Path $repo $file)).Hash -ne $hashes[$file]) { Fail "turbo $Step changed $file" }
  }
}

# Fails unless the stand-in tools logged each call since the last check.
function Assert-Called([string[]]$Expected) {
  $log = [IO.File]::ReadAllText($calls)
  [IO.File]::WriteAllText($calls, '')
  foreach ($call in $Expected) {
    if (-not (Test-Contains $log $call)) { Fail "no managed `"$call`" call: $log" }
  }
}

# Each native frozen installer ran with its managed tool, plus $Extra calls.
function Assert-Installs([string]$Text, [string[]]$Extra = @()) {
  $reports = @('javascript dependencies: installed; pnpm-lock.yaml unchanged', 'python dependencies: installed; uv.lock unchanged',
    'rust dependencies: installed; Cargo.lock unchanged', 'go dependencies: installed; go.sum unchanged')
  foreach ($report in $reports) {
    if (-not (Test-Contains $Text $report)) { Fail "setup did not report `"$report`": $Text" }
  }
  Assert-Called (@('pnpm install --frozen-lockfile', 'uv sync --locked --all-packages', 'cargo fetch --locked', 'go mod download') + $Extra)
}

# Plain turbo run: every task uses its managed tools (from apps\web, turbo
# infers only that package). The JavaScript task's node.exe is the managed x64
# Node, first on PATH in its plain (non-verbatim) spelling, and strict mode
# filters the caller's variables while loose mode passes them.
function Assert-Build([string]$Directory, [string]$Mode, [string]$Secret) {
  $result = Assert-Succeeds $Directory @('run', 'build', "--env-mode=$Mode") -Offline
  $expected = @('fixture-pnpm 10.0.0 run build', "smoke-tool 1.2.3 secret=$Secret")
  if ($Directory -eq $repo) {
    $expected += @("managed-cargo build --package=rs-app --locked toolchain=1.99.0-$triple secret=$Secret",
      "managed-uv build --package=py-app python=cpython-3.12.12-windows-x86_64-none downloads=never secret=$Secret",
      "managed-go build ./... GOTOOLCHAIN=local goroot=locked secret=$Secret")
  }
  foreach ($line in $expected) {
    if (-not (Test-Contains $result.Text $line)) { Fail "turbo run --env-mode=$Mode did not print `"$line`": $($result.Text)" }
  }
  $line = @($result.Text -split "`r?`n" | Where-Object { $_ -match 'managed-probe (.*)$' })[0]
  $node, $reported, $arch, $first, $seen = ($line -replace '^.*managed-probe ', '').Split('|')
  $managed = [regex]::Escape('\.turbo\tools\')
  if ($node -notmatch "$managed.*\\node\.exe$" -or $reported -ne "v$nodeVersion" -or $arch -ne 'x64' -or
    $first -notmatch $managed -or $first.StartsWith('\\?\') -or $seen -ne $Secret) {
    Fail "unexpected $Mode task environment: $line $($result.Text)"
  }
  Assert-Unchanged 'run'
  return $first
}

# Owner-only, protected descriptors, read from the live objects.
function Assert-OwnerOnly([string]$Path) {
  $me = [Security.Principal.WindowsIdentity]::GetCurrent().User
  $sid = [Security.Principal.SecurityIdentifier]
  $acl = Get-Acl -LiteralPath $Path
  if (-not $acl.AreAccessRulesProtected -or $acl.GetOwner($sid) -ne $me) { Fail "$Path is not owned and protected for $me" }
  foreach ($rule in $acl.GetAccessRules($true, $true, $sid)) {
    if ($rule.AccessControlType -eq 'Allow' -and $rule.IdentityReference -ne $me) { Fail "$Path grants $($rule.IdentityReference)" }
  }
}

function Join-Lines([string[]]$Lines) { return ($Lines -join "`n") + "`n" }

# The journey repository: Node.js and pnpm, a Cargo workspace, a uv workspace,
# a Go workspace, and the additional tools declared in $Tools.
function Write-Repository([string]$NodeVersion, [string]$Tools) {
  $flags = '"experimentalSetup":true,"experimentalCargoWorkspaces":true,"experimentalPythonWorkspaces":true,"experimentalGoWorkspaces":true'
  $files = @{
    'turbo.json' = "{`"futureFlags`":{$flags},`"agentGuidance`":false,`"setup`":{`"additionalTools`":$Tools},`"tasks`":{`"build`":{`"cache`":false}}}"
    '.gitignore' = Join-Lines '/.turbo/', 'node_modules/', '/target/', '.venv/'
    '.nvmrc' = Join-Lines $NodeVersion
    'package.json' = '{"name":"journey","private":true,"packageManager":"pnpm@10.0.0"}'
    'pnpm-workspace.yaml' = Join-Lines "packages: ['apps/*']"
    'pnpm-lock.yaml' = Join-Lines "lockfileVersion: '9.0'", 'importers:', '  .: {}', '  apps/web: {}'
    'apps\web\package.json' = '{"name":"web","scripts":{"build":"smoke-tool && node probe.cjs"}}'
    'apps\web\probe.cjs' = Join-Lines 'const first = process.env.PATH.split(require("node:path").delimiter)[0];',
      'const node = require("node:fs").realpathSync(process.execPath);', 'const secret = process.env.SMOKE_SECRET ?? "filtered";',
      'console.log(`managed-probe ${node}|${process.version}|${process.arch}|${first}|${secret}`);'
    'Cargo.toml' = Join-Lines '[workspace]', 'members = ["crates/rs-app"]', 'resolver = "2"', '', '[workspace.metadata]', 'name = "rs-root"'
    'rust-toolchain.toml' = Join-Lines '[toolchain]', 'channel = "stable"', 'profile = "minimal"'
    'Cargo.lock' = Join-Lines 'version = 4', '', '[[package]]', 'name = "rs-app"', 'version = "0.1.0"'
    'crates\rs-app\Cargo.toml' = Join-Lines '[package]', 'name = "rs-app"', 'version = "0.1.0"', 'edition = "2021"'
    'crates\rs-app\src\lib.rs' = ''
    'pyproject.toml' = Join-Lines '[tool.turbo]', 'name = "py-root"', '', '[tool.uv]', 'required-version = ">=0.13"', '',
      '[tool.uv.workspace]', 'members = ["packages/*"]'
    '.python-version' = Join-Lines '3.12'
    'packages\py-app\pyproject.toml' = Join-Lines '[project]', 'name = "py-app"', 'version = "0.1.0"', 'requires-python = ">=3.12"',
      'dependencies = []', '', '[build-system]', 'requires = ["uv_build>=0.8,<2"]', 'build-backend = "uv_build"'
    'packages\py-app\src\py_app\__init__.py' = ''
    'uv.lock' = Join-Lines 'version = 1', 'revision = 3', 'requires-python = ">=3.12"', '', '[manifest]', 'members = ["py-app"]', '',
      '[[package]]', 'name = "py-app"', 'version = "0.1.0"', 'source = { editable = "packages/py-app" }'
    'go.work' = Join-Lines 'go 1.22', '', 'use ./goapp'
    'goapp\go.mod' = Join-Lines 'module example.com/goapp', '', 'go 1.22'
    'goapp\main.go' = Join-Lines 'package main', '', 'func main() {}'
  }
  foreach ($name in $files.Keys) {
    $path = Join-Path $repo $name
    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($path)) | Out-Null
    [IO.File]::WriteAllText($path, $files[$name])
  }
}

try {
  [IO.Directory]::CreateDirectory($work) | Out-Null
  $install = Join-Path $work 'install'
  $assets = (Resolve-Path -LiteralPath $Assets).Path
  foreach ($name in @($archiveName, 'SHA256SUMS')) {
    if (-not (Test-Path -LiteralPath (Join-Path $assets $name))) { Fail "missing $name in $assets" }
  }

  $mirrorNode = (Get-Command node.exe -ErrorAction SilentlyContinue).Source
  $system = $env:SystemRoot
  $env:Path = "$system\System32;$system;$system\System32\Wbem;$system\System32\WindowsPowerShell\v1.0"
  $found = @('node', 'npm', 'npx', 'pnpm', 'yarn', 'corepack', 'bun', 'turbo', 'cargo', 'rustc', 'rustup', 'uv', 'python', 'python3', 'go') |
    Where-Object { Get-Command $_ -ErrorAction SilentlyContinue }
  if ($found) { Fail "found $found on the smoke PATH" }
  $temp = Join-Path $work 'tmp'
  [IO.Directory]::CreateDirectory($temp) | Out-Null
  $closed = 'http://127.0.0.1:9'
  $environment = @{
    SystemRoot = $system; windir = $system; ComSpec = $env:ComSpec; SystemDrive = $env:SystemDrive
    PATHEXT = '.COM;.EXE;.BAT;.CMD'; PATH = $env:Path; TEMP = $temp; TMP = $temp
    USERPROFILE = $env:USERPROFILE; APPDATA = $env:APPDATA; LOCALAPPDATA = $env:LOCALAPPDATA
    TURBO_TELEMETRY_DISABLED = '1'; DO_NOT_TRACK = '1'; TURBO_NO_UPDATE_NOTIFIER = '1'
    HTTP_PROXY = $closed; HTTPS_PROXY = $closed; ALL_PROXY = $closed; NO_PROXY = '127.0.0.1'
    SMOKE_SECRET = 'present'
  }

  $requestLog = Join-Path $work 'requests.log'
  [IO.File]::WriteAllText($requestLog, '')
  $repo = Join-Path $work 'repo'
  $web = Join-Path $repo 'apps\web'

  # Architecture from the system environment, independent of either probe.
  $environmentKey = 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Environment'
  if ((Get-ItemProperty -LiteralPath $environmentKey).PROCESSOR_ARCHITECTURE -eq 'ARM64') {
    $failure = ''
    try { $null = Install-Turbo $assets -Hook } catch { $failure = $_.Exception.Message }
    if (-not ($failure.Contains('not available for Windows arm64') -and $failure.Contains('WSL'))) { Fail "the installer did not refuse arm64: $failure" }
    if ($script:Requests -ne 0 -or (Test-Path -LiteralPath $install) -or (Test-Path -LiteralPath $profilePath)) {
      Fail 'the installer downloaded or wrote before refusing arm64'
    }
    # The x64 binary under emulation, extracted from the packaged archive,
    # refuses every ecosystem of the journey repository.
    $extracted = Join-Path $work 'x64'
    [IO.Directory]::CreateDirectory($extracted) | Out-Null
    & tar.exe -xzf (Join-Path $assets $archiveName) -C $extracted
    if ($LASTEXITCODE -ne 0) { Fail "could not extract $archiveName" }
    $turbo = Join-Path $extracted 'turbo.exe'
    $tool = '{"smoke-tool":{"version":"1.2.3","artifacts":{"windows-x64":{"url":"https://downloads.example.test/smoke-tool.tar.gz",' +
      "`"sha256`":`"$('0' * 64)`",`"format`":`"tar-gz`",`"rootPrefix`":`"smoke-tool-1.2.3`",`"executables`":{`"smoke-tool`":`"bin/smoke-tool.exe`"}}}}}"
    Write-Repository '24.0.0' $tool
    foreach ($arguments in @(@('setup'), @('setup', '--tools-only'), @('setup', '--check'))) {
      $result = Invoke-Turbo $web $arguments
      if ($result.Code -eq 0 -or -not (Test-Contains $result.Text 'not qualified for Windows arm64')) { Fail "turbo $arguments did not refuse arm64: $($result.Text)" }
    }
    if ((Test-Path -LiteralPath (Join-Path $repo 'turbo.lock')) -or (Test-Path -LiteralPath (Join-Path $repo '.turbo'))) { Fail 'setup wrote before refusing arm64' }
    Write-Output "Standalone Windows smoke passed: turbo $Version refuses arm64 in the installer and in setup for every ecosystem."
    return
  }

  # Integrity: the packaged archive with appended bytes must not install.
  $tampered = Join-Path $work 'tampered'
  [IO.Directory]::CreateDirectory($tampered) | Out-Null
  Copy-Item -LiteralPath (Join-Path $assets 'SHA256SUMS') -Destination $tampered
  Copy-Item -LiteralPath (Join-Path $assets $archiveName) -Destination $tampered
  $stream = [IO.File]::Open((Join-Path $tampered $archiveName), 'Append')
  $stream.WriteByte(0)
  $stream.Dispose()
  $failure = ''
  try { $null = Install-Turbo $tampered } catch { $failure = $_.Exception.Message }
  if (-not $failure.Contains("SHA-256 verification failed for $archiveName")) { Fail "unexpected result for a tampered archive: $failure" }
  if (Test-Path -LiteralPath $install) { Fail 'a tampered archive changed the install directory' }
  $turbo = Join-Path $install 'turbo.exe'
  $output = Install-Turbo $assets -Hook
  if (-not ($output.Contains("Installed turbo $Version at $turbo") -and $output.Contains("Added the turbo shell hook to $profilePath"))) {
    Fail "unexpected install output: $output"
  }
  if (-not [IO.File]::ReadAllText($profilePath).Contains("& '$turbo' hook powershell")) { Fail "the profile has no turbo hook: $([IO.File]::ReadAllText($profilePath))" }
  $reported = (Invoke-Turbo $work @('--version')).Text.Trim()
  if ($reported -ne $Version) { Fail "turbo --version reported $reported, expected $Version" }

  if (-not $mirrorNode) { Fail 'the fixture mirror needs Node.js on the runner' }
  $state = Join-Path $work 'mirror'
  [IO.Directory]::CreateDirectory($state) | Out-Null
  $mirrorLog = Join-Path $work 'mirror.log'
  $mirror = Start-Process -FilePath $mirrorNode -PassThru -NoNewWindow -RedirectStandardOutput (Join-Path $work 'mirror.out') -RedirectStandardError $mirrorLog -ArgumentList @(
    (ConvertTo-Argument (Join-Path $PSScriptRoot 'setup-fixture-mirror.mjs')), (ConvertTo-Argument $state))
  $ready = Join-Path $state 'ready'
  for ($tries = 0; -not (Test-Path -LiteralPath $ready) -or (Get-Item -LiteralPath $ready).Length -eq 0; $tries++) {
    if ($mirror.HasExited -or $tries -gt 600) { Fail "the fixture mirror did not start: $(Get-Content -Raw -LiteralPath $mirrorLog -ErrorAction SilentlyContinue)" }
    Start-Sleep -Milliseconds 100
  }
  $port, $nodeVersion = ([IO.File]::ReadAllText($ready).Trim()).Split(' ')
  $requestLog = Join-Path $state 'requests.log'
  $calls = Join-Path $state 'calls.log'
  $base = "https://127.0.0.1:$port"
  $environment.SSL_CERT_FILE = Join-Path $PSScriptRoot 'fixtures\setup-mirror\ca.pem'
  $environment.TURBO_SETUP_NODE_MIRROR = "$base/node"
  $environment.TURBO_SETUP_NPM_REGISTRY = "$base/npm"
  $environment.TURBO_SETUP_RUST_MIRROR = "$base/rust"
  $environment.TURBO_SETUP_GO_MIRROR = "$base/golang"
  $environment.TURBO_SETUP_UV_MIRROR = "$base/astral"
  $environment.TURBO_SETUP_PYTHON_MIRROR = "$base/astral"
  $environment.TURBO_SETUP_ARTIFACT_MIRROR = "$base/artifacts"
  Write-Repository $nodeVersion ([IO.File]::ReadAllText((Join-Path $state 'additional-tools.json')))

  # Tampered mirror bytes, while the shared download cache is still empty:
  # the new lock is written, but nothing is installed.
  [IO.File]::WriteAllText((Join-Path $state 'tamper'), '')
  $result = Invoke-Turbo $web @('setup', '--tools-only')
  Remove-Item -LiteralPath (Join-Path $state 'tamper')
  if ($result.Code -eq 0 -or -not (Test-Contains $result.Text 'does not match the trusted digest')) { Fail "setup accepted a tampered artifact: $($result.Text)" }
  if (Test-Path -LiteralPath (Join-Path $repo '.turbo\tools\manifest.json')) { Fail 'a tampered artifact was installed' }
  if (-not (Test-Contains $result.Text 'Commit turbo.lock')) { Fail "setup did not ask to commit the new lock: $($result.Text)" }
  [IO.File]::WriteAllText($calls, '')

  # Setup from a subdirectory through the mirror only: every tool, then every
  # ecosystem's locked dependencies.
  $records = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'turborepo\setup-roots'
  $known = @(Get-ChildItem -LiteralPath $records -Filter *.json -ErrorAction SilentlyContinue | ForEach-Object { $_.Name })
  $result = Assert-Succeeds $web @('setup')
  if ($result.Requests -eq 0) { Fail 'setup made no mirror requests' }
  Assert-Installs $result.Text @('rustup toolchain install 1.99.0')
  $lock = [IO.File]::ReadAllText((Join-Path $repo 'turbo.lock'))
  if ($lock.Contains('127.0.0.1')) { Fail 'turbo.lock records the mirror location' }
  foreach ($tool in @('node', 'pnpm', 'rustup', 'rust', 'uv', 'python', 'go', 'smoke-tool')) {
    if ($lock -notmatch "`"$tool`": *\{") { Fail "turbo.lock does not lock ${tool}: $lock" }
  }
  $hashes = @{}
  foreach ($file in @('turbo.lock', '.turbo\tools\manifest.json')) { $hashes[$file] = (Get-FileHash -LiteralPath (Join-Path $repo $file)).Hash }

  # The trusted record is new, owner-only, and records the plain bin spelling.
  $record = @(Get-ChildItem -LiteralPath $records -Filter *.json | Where-Object { $known -notcontains $_.Name })
  if ($record.Count -ne 1) { Fail "expected one new trusted record in $records" }
  Assert-OwnerOnly $records
  Assert-OwnerOnly $record[0].FullName
  $recorded = (Get-Content -Raw -LiteralPath $record[0].FullName | ConvertFrom-Json).installation.bin
  if ($recorded.StartsWith('\\?\')) { Fail "the trusted record uses a verbatim bin path: $recorded" }

  foreach ($directory in @($repo, $web)) {
    $bin = Assert-Build $directory 'strict' 'filtered'
    $null = Assert-Build $directory 'loose' 'present'
  }
  if ($bin -ne $recorded) { Fail "run activated $bin, but setup recorded $recorded" }
  # turbo's own Cargo, uv, and Go discovery used the managed tools, too.
  Assert-Called @('cargo metadata', 'rustc -vV', 'uv --version', 'uv python find', 'go work edit -json', 'go env -json')

  # Repeats reuse the tools without network and rerun the locked installers;
  # frozen CI does the same. Offline setup prepares tools only, and offline
  # `--check` reports readiness.
  $result = Assert-Succeeds $repo @('setup') -Offline
  Assert-Installs $result.Text
  $result = Assert-Succeeds $repo @('setup') @{ CI = '1' } -Offline
  Assert-Installs $result.Text
  $null = Assert-Succeeds $repo @('setup', '--tools-only', '--offline') -Offline
  $null = Assert-Succeeds $repo @('setup', '--check', '--offline') -Offline
  Assert-Unchanged 'setup'

  # The installer's hook in a new Windows PowerShell whose PATH gained the
  # install directory: inside the repository it puts exactly the recorded bin
  # first, with the variables that Rust's rustup proxies and Go's hard-linked
  # go.exe need, and leaving restores PATH exactly and removes them. Without a
  # console PowerShell never draws a prompt, so each step calls it.
  $hookEnvironment = $environment.Clone()
  $hookEnvironment.Remove('SMOKE_SECRET')
  $hookEnvironment.PATH = "$install;$($environment.PATH)"
  $steps = @(". '$profilePath'", 'Show start $env:PATH', "cd '$web'", 'Show inside $env:PATH', 'Show node (node -p process.version)',
    'Show tool (smoke-tool)', 'Show cargo (cargo build)', 'Show go (go build)', "cd '$work'", 'Show outside $env:PATH',
    'Show left "$env:RUSTUP_HOME|$env:RUSTUP_TOOLCHAIN|$env:GOROOT|$env:GOTOOLCHAIN"')
  $hookInput = (@('function Show($key, $value) { "$key=$value" }') + @($steps | ForEach-Object { "$_`n`$null = prompt" }) + 'exit') -join "`n"
  $before = @(Get-Content -LiteralPath $requestLog).Count
  $powershell = "$system\System32\WindowsPowerShell\v1.0\powershell.exe"
  $result = Start-Captured $powershell '-NoProfile -NoLogo -NonInteractive -Command -' $work @($hookEnvironment) $hookInput
  $values = @{}
  foreach ($line in ($result.Text -split "`r?`n")) {
    if ($line -match '^(\w+)=(.*)$' -and -not $values.ContainsKey($Matches[1])) { $values[$Matches[1]] = $Matches[2] }
  }
  $start = $hookEnvironment.PATH
  if ($result.Code -ne 0 -or $values.start -ne $start -or $values.inside -ne "$recorded;$start" -or $values.outside -ne $start -or
    $values.node -ne "v$nodeVersion" -or $values.tool -ne 'smoke-tool 1.2.3 secret=filtered' -or
    $values.cargo -ne "managed-cargo build toolchain=1.99.0-$triple secret=filtered" -or
    $values.go -ne 'managed-go build GOTOOLCHAIN=local goroot=locked secret=filtered' -or $values.left -ne '|||') {
    Fail "the PowerShell hook did not activate and restore the managed tools: $($result.Text)"
  }
  if (@(Get-Content -LiteralPath $requestLog).Count -ne $before) { Fail 'the PowerShell hook made mirror requests' }
  Assert-Unchanged 'hook'

  # turbo exec runs arbitrary commands with the managed tools, from the
  # caller's directory, with exact arguments, stdin, and exit codes, and finds
  # managed .cmd launchers through PATHEXT.
  $script = 'console.log("exec-probe " + JSON.stringify([require("fs").realpathSync(process.execPath), process.cwd(), process.argv.slice(1)]))'
  $words = @('a b', '', '100%', 'q"uote', 'back\slash\', '%PATH%', '^&|<>')
  $result = Assert-Succeeds $web (@('exec', '--', 'node', '-e', $script) + $words) -Offline
  $node, $cwd, $seen = (($result.Text -split "`r?`n" | Where-Object { $_ -like 'exec-probe *' }) -replace '^exec-probe ', '' | ConvertFrom-Json)
  if ($node -notmatch [regex]::Escape('\.turbo\tools\') -or $cwd -ne $web -or (@($seen) -join '|') -cne ($words -join '|')) {
    Fail "turbo exec ran with the wrong node, directory, or arguments: $($result.Text)"
  }
  $result = Assert-Succeeds $web @('exec', '--', 'node', '-e', 'process.stdin.pipe(process.stdout)') -Stdin 'piped stdin' -Offline
  if (-not $result.Text.Contains('piped stdin')) { Fail "turbo exec did not forward stdin: $($result.Text)" }
  $result = Invoke-Turbo $web @('exec', '--', 'node', '-e', 'process.exit(7)')
  if ($result.Code -ne 7) { Fail "turbo exec exited $($result.Code), expected 7: $($result.Text)" }
  $result = Assert-Succeeds $web @('exec', '--', 'pnpm', 'run', 'build') -Offline
  if (-not ($result.Text.Contains('fixture-pnpm 10.0.0 run build') -and $result.Text.Contains("|v$nodeVersion|x64|"))) { Fail "turbo exec did not run managed pnpm.cmd: $($result.Text)" }
  $result = Invoke-Turbo $web @('exec', '--', 'pnpx')
  if ($result.Code -ne 97) { Fail "turbo exec did not pass through pnpx.cmd's exit code: $($result.Code) $($result.Text)" }
  # Every other managed tool, including python.cmd's launcher.
  $tools = @(
    @("toolchain=1.99.0-$triple", 'cargo', 'build'), @('uv 0.13.0', 'uv', '--version'), @('Python 3.12.12', 'python', '--version'),
    @('go version go1.22.12', 'go', 'version'), @('smoke-tool 1.2.3 secret=present', 'smoke-tool'))
  foreach ($tool in $tools) {
    $command = @($tool | Select-Object -Skip 1)
    $result = Assert-Succeeds $web (@('exec', '--') + $command) -Offline
    if (-not (Test-Contains $result.Text $tool[0])) { Fail "turbo exec -- $command did not run the managed tool: $($result.Text)" }
  }
  Assert-Unchanged 'exec'

  # A clone with the committed lock but no installation: exec never installs.
  $clone = Join-Path $work 'clone'
  [IO.Directory]::CreateDirectory($clone) | Out-Null
  Get-ChildItem -LiteralPath $repo -Force | Where-Object { $_.Name -ne '.turbo' } | Copy-Item -Destination $clone -Recurse
  $result = Invoke-Turbo $clone @('exec', '--', 'node', '-e', 'console.log("exec-ran")')
  if ($result.Code -eq 0 -or -not (Test-Contains $result.Text 'turbo setup') -or $result.Text.Contains('exec-ran') -or $result.Requests -ne 0) {
    Fail "turbo exec ran, downloaded, or gave no guidance without an installation: $($result.Text)"
  }
  if ((Test-Path -LiteralPath (Join-Path $clone '.turbo\tools')) -or (Get-FileHash -LiteralPath (Join-Path $clone 'turbo.lock')).Hash -ne $hashes['turbo.lock']) {
    Fail 'turbo exec wrote an installation or changed turbo.lock'
  }

  # Tamper: a deleted launcher and changed node.exe bytes each stop run before
  # any task and fail --check; setup repairs both.
  $damage = @(
    { Remove-Item -LiteralPath (Join-Path $bin 'pnpm.cmd') },
    { $node = Join-Path $bin 'node.exe'; Set-ItemProperty -LiteralPath $node -Name IsReadOnly -Value $false; [IO.File]::AppendAllText($node, 'tampered') }
  )
  foreach ($change in $damage) {
    & $change
    $result = Invoke-Turbo $repo @('run', 'build')
    if ($result.Code -eq 0 -or -not (Test-Contains $result.Text 'turbo setup') -or $result.Text.Contains('managed-probe')) { Fail "run accepted a damaged installation: $($result.Text)" }
    $result = Invoke-Turbo $repo @('exec', '--', 'node', '-e', 'console.log("exec-ran")')
    if ($result.Code -eq 0 -or -not (Test-Contains $result.Text 'turbo setup') -or $result.Text.Contains('exec-ran') -or $result.Requests -ne 0) { Fail "exec accepted a damaged installation: $($result.Text)" }
    if ((Invoke-Turbo $repo @('setup', '--check', '--tools-only', '--offline')).Code -eq 0) { Fail 'setup --check accepted a damaged installation' }
    $null = Assert-Succeeds $repo @('setup', '--tools-only')
    if ((Get-FileHash -LiteralPath (Join-Path $repo 'turbo.lock')).Hash -ne $hashes['turbo.lock']) { Fail 'repair changed turbo.lock' }
    $hashes['.turbo\tools\manifest.json'] = (Get-FileHash -LiteralPath (Join-Path $repo '.turbo\tools\manifest.json')).Hash
    $bin = Assert-Build $repo 'strict' 'filtered'
  }

  Write-Output ("Standalone Windows smoke passed for turbo $Version (x64): Node.js $nodeVersion, pnpm 10.0.0, Rust 1.99.0, uv 0.13.0, " +
    'Python 3.12.12, Go 1.22.12, smoke-tool 1.2.3, locked dependencies, and the PowerShell hook.')
} finally {
  $env:Path = $previousPath
  if ($mirror -and -not $mirror.HasExited) { Stop-Process -Id $mirror.Id -Force }
  if (Test-Path -LiteralPath $work) { Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue }
}
