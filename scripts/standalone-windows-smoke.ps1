# Release smoke for the packaged native Windows standalone turbo. It installs
# the archive with install.ps1 from staged release assets while Node.js and
# JavaScript package managers are absent from PATH. On x64 it then runs the
# managed setup journey against the loopback fixture mirror
# (setup-fixture-mirror.mjs): setup, plain `turbo run` in strict and loose
# modes with managed x64 Node.js and pnpm, repeats, `--check --offline`, the
# owner-only trusted record, and tamper repair. On an arm64 host, the installer
# and setup must refuse instead. turbo runs with a cleared environment and an
# unreachable proxy; only the mirror answers.
#
# Usage: scripts/standalone-windows-smoke.ps1 -Version <version> -Binary <turbo.exe>
# The fixture mirror runs on the runner's Node.js, outside turbo's PATH.
param(
  [Parameter(Mandatory = $true)][string]$Version,
  [Parameter(Mandatory = $true)][string]$Binary
)
$ErrorActionPreference = 'Stop'
$installerPath = Join-Path $PSScriptRoot '..\apps\docs\public\install.ps1'
$archiveName = "turbo-$Version-x86_64-pc-windows-msvc.tar.gz"
$root = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [IO.Path]::GetTempPath() }
$work = Join-Path $root ('turbo-windows-smoke-' + [Guid]::NewGuid().ToString('N'))
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

function Install-Turbo([string]$Directory) {
  $script:Served = $Directory
  return (. $installerPath -Version $Version -InstallDirectory $install -NoModifyPath 6>&1 | ForEach-Object { [string]$_ }) -join "`n"
}

# CommandLineToArgvW quoting, so arguments reach turbo exactly.
function ConvertTo-Argument([string]$Value) {
  if ($Value -ne '' -and $Value -notmatch '[\s"]') { return $Value }
  return '"' + (($Value -replace '(\\*)"', '$1$1\"') -replace '(\\+)$', '$1$1') + '"'
}

# Runs turbo with only $environment, like `env -i`. Sets Code, Text, Requests.
function Invoke-Turbo([string]$Directory, [string[]]$Arguments, [hashtable]$Extra = @{}) {
  $before = @(Get-Content -LiteralPath $requestLog).Count
  $info = New-Object Diagnostics.ProcessStartInfo
  $info.FileName = $turbo
  $info.Arguments = ($Arguments | ForEach-Object { ConvertTo-Argument $_ }) -join ' '
  $info.WorkingDirectory = $Directory
  $info.UseShellExecute = $false
  $info.RedirectStandardOutput = $true
  $info.RedirectStandardError = $true
  $info.EnvironmentVariables.Clear()
  foreach ($variables in @($environment, $Extra)) {
    foreach ($key in $variables.Keys) { $info.EnvironmentVariables[$key] = $variables[$key] }
  }
  $process = [Diagnostics.Process]::Start($info)
  $stdout = $process.StandardOutput.ReadToEndAsync()
  $stderr = $process.StandardError.ReadToEndAsync()
  $process.WaitForExit()
  return [pscustomobject]@{
    Code = $process.ExitCode
    Text = $stdout.Result + $stderr.Result
    Requests = @(Get-Content -LiteralPath $requestLog).Count - $before
  }
}

function Assert-Succeeds([string]$Directory, [string[]]$Arguments, [hashtable]$Extra = @{}, [switch]$Offline) {
  $result = Invoke-Turbo $Directory $Arguments $Extra
  if ($result.Code -ne 0) { Fail "turbo $Arguments exited $($result.Code): $($result.Text)" }
  if ($Offline -and $result.Requests -ne 0) { Fail "turbo $Arguments made $($result.Requests) mirror requests: $($result.Text)" }
  return $result
}

function Assert-Unchanged([string]$Step) {
  foreach ($file in @('turbo.lock', '.turbo\tools\manifest.json')) {
    if ((Get-FileHash -LiteralPath (Join-Path $repo $file)).Hash -ne $hashes[$file]) { Fail "turbo $Step changed $file" }
  }
}

# Plain turbo run: managed pnpm.cmd dispatches, and the task's node.exe is the
# managed x64 Node, first on PATH in its plain (non-verbatim) spelling.
function Assert-Build([string]$Directory, [string]$Mode, [string]$Secret) {
  $result = Assert-Succeeds $Directory @('run', 'build', "--env-mode=$Mode") -Offline
  if (-not (Test-Contains $result.Text 'fixture-pnpm 10.0.0 run build')) { Fail "managed pnpm did not run: $($result.Text)" }
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

try {
  [IO.Directory]::CreateDirectory($work) | Out-Null
  $install = Join-Path $work 'install'
  $assets = Join-Path $work 'assets'
  [IO.Directory]::CreateDirectory($assets) | Out-Null
  $staged = Join-Path $work 'staged'
  [IO.Directory]::CreateDirectory($staged) | Out-Null
  Copy-Item -LiteralPath $Binary -Destination (Join-Path $staged 'turbo.exe')
  & tar.exe -czf (Join-Path $assets $archiveName) -C $staged turbo.exe
  if ($LASTEXITCODE -ne 0) { Fail "could not package $Binary" }
  $digest = (Get-FileHash -LiteralPath (Join-Path $assets $archiveName) -Algorithm SHA256).Hash.ToLowerInvariant()
  [IO.File]::WriteAllText((Join-Path $assets 'SHA256SUMS'), "$digest  $archiveName`n")

  $mirrorNode = (Get-Command node.exe -ErrorAction SilentlyContinue).Source
  $system = $env:SystemRoot
  $env:Path = "$system\System32;$system;$system\System32\Wbem;$system\System32\WindowsPowerShell\v1.0"
  $found = @('node', 'npm', 'npx', 'pnpm', 'yarn', 'corepack', 'bun', 'turbo') | Where-Object { Get-Command $_ -ErrorAction SilentlyContinue }
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
  [IO.Directory]::CreateDirectory($web) | Out-Null

  # Architecture from the system environment, independent of either probe.
  $environmentKey = 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Environment'
  if ((Get-ItemProperty -LiteralPath $environmentKey).PROCESSOR_ARCHITECTURE -eq 'ARM64') {
    $failure = ''
    try { $null = Install-Turbo $assets } catch { $failure = $_.Exception.Message }
    if (-not ($failure.Contains('not available for Windows arm64') -and $failure.Contains('WSL'))) { Fail "the installer did not refuse arm64: $failure" }
    if ($script:Requests -ne 0 -or (Test-Path -LiteralPath $install)) { Fail 'the installer downloaded or wrote before refusing arm64' }
    $turbo = $Binary
    [IO.File]::WriteAllText((Join-Path $repo 'turbo.json'), '{"futureFlags":{"experimentalSetup":true}}')
    [IO.File]::WriteAllText((Join-Path $repo '.nvmrc'), "24.0.0`n")
    foreach ($arguments in @(@('setup', '--tools-only'), @('setup', '--check', '--tools-only'))) {
      $result = Invoke-Turbo $web $arguments
      if ($result.Code -eq 0 -or -not (Test-Contains $result.Text 'not qualified for Windows arm64')) { Fail "turbo $arguments did not refuse arm64: $($result.Text)" }
    }
    if ((Test-Path -LiteralPath (Join-Path $repo 'turbo.lock')) -or (Test-Path -LiteralPath (Join-Path $repo '.turbo'))) { Fail 'setup wrote before refusing arm64' }
    Write-Output "Standalone Windows smoke passed: turbo $Version refuses arm64 in the installer and setup."
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
  $output = Install-Turbo $assets
  if (-not $output.Contains("Installed turbo $Version at $turbo")) { Fail "unexpected install output: $output" }
  $reported = (Invoke-Turbo $work @('--version')).Text.Trim()
  if ($reported -ne $Version) { Fail "turbo --version reported $reported, expected $Version" }

  if (-not $mirrorNode) { Fail 'the fixture mirror needs Node.js on the runner' }
  $state = Join-Path $work 'mirror'
  [IO.Directory]::CreateDirectory($state) | Out-Null
  $mirror = Start-Process -FilePath $mirrorNode -PassThru -WindowStyle Hidden -ArgumentList @(
    (ConvertTo-Argument (Join-Path $PSScriptRoot 'setup-fixture-mirror.mjs')), (ConvertTo-Argument $state))
  $ready = Join-Path $state 'ready'
  for ($tries = 0; -not (Test-Path -LiteralPath $ready) -or (Get-Item -LiteralPath $ready).Length -eq 0; $tries++) {
    if ($mirror.HasExited -or $tries -gt 600) { Fail 'the fixture mirror did not start' }
    Start-Sleep -Milliseconds 100
  }
  $port, $nodeVersion = ([IO.File]::ReadAllText($ready).Trim()).Split(' ')
  $requestLog = Join-Path $state 'requests.log'
  $environment.SSL_CERT_FILE = Join-Path $PSScriptRoot 'fixtures\setup-mirror\ca.pem'
  $environment.TURBO_SETUP_NODE_MIRROR = "https://127.0.0.1:$port/node"
  $environment.TURBO_SETUP_NPM_REGISTRY = "https://127.0.0.1:$port/npm"

  $files = @{
    'turbo.json' = '{"futureFlags":{"experimentalSetup":true},"agentGuidance":false,"tasks":{"build":{"cache":false}}}'
    '.nvmrc' = "$nodeVersion`n"
    '.gitignore' = "/.turbo/`nnode_modules/`n"
    'package.json' = '{"name":"journey","private":true,"packageManager":"pnpm@10.0.0"}'
    'pnpm-workspace.yaml' = "packages: ['apps/*']`n"
    'pnpm-lock.yaml' = "lockfileVersion: '9.0'`nimporters:`n  .: {}`n  apps/web: {}`n"
    'apps\web\package.json' = '{"name":"web","scripts":{"build":"node probe.cjs"}}'
    'apps\web\probe.cjs' = @'
const first = process.env.PATH.split(require("node:path").delimiter)[0];
const node = require("node:fs").realpathSync(process.execPath);
const secret = process.env.SMOKE_SECRET ?? "filtered";
console.log(`managed-probe ${node}|${process.version}|${process.arch}|${first}|${secret}`);
'@
  }
  foreach ($name in $files.Keys) { [IO.File]::WriteAllText((Join-Path $repo $name), $files[$name]) }

  # Tampered mirror bytes: the new lock is written, but nothing is installed.
  [IO.File]::WriteAllText((Join-Path $state 'tamper'), '')
  $result = Invoke-Turbo $web @('setup', '--tools-only')
  Remove-Item -LiteralPath (Join-Path $state 'tamper')
  if ($result.Code -eq 0 -or -not (Test-Contains $result.Text 'does not match the trusted digest')) { Fail "setup accepted a tampered artifact: $($result.Text)" }
  if (Test-Path -LiteralPath (Join-Path $repo '.turbo\tools\manifest.json')) { Fail 'a tampered artifact was installed' }
  if (-not (Test-Contains $result.Text 'Commit turbo.lock')) { Fail "setup did not ask to commit the new lock: $($result.Text)" }

  $records = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'turborepo\setup-roots'
  $known = @(Get-ChildItem -LiteralPath $records -Filter *.json -ErrorAction SilentlyContinue | ForEach-Object { $_.Name })
  $result = Assert-Succeeds $web @('setup', '--tools-only')
  if ($result.Requests -eq 0) { Fail 'setup made no mirror requests' }
  if ([IO.File]::ReadAllText((Join-Path $repo 'turbo.lock')).Contains('127.0.0.1')) { Fail 'turbo.lock records the mirror location' }
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

  # Repeats reuse the installation without network, including frozen CI.
  $result = Assert-Succeeds $repo @('setup') -Offline
  if (-not (Test-Contains $result.Text 'Dependencies skipped')) { Fail "setup did not report skipped dependencies: $($result.Text)" }
  $null = Assert-Succeeds $repo @('setup', '--tools-only') @{ CI = '1' } -Offline
  $null = Assert-Succeeds $repo @('setup', '--tools-only', '--offline') -Offline
  $null = Assert-Succeeds $repo @('setup', '--check', '--tools-only', '--offline') -Offline
  Assert-Unchanged 'setup'

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
    if ((Invoke-Turbo $repo @('setup', '--check', '--tools-only', '--offline')).Code -eq 0) { Fail 'setup --check accepted a damaged installation' }
    $null = Assert-Succeeds $repo @('setup', '--tools-only')
    if ((Get-FileHash -LiteralPath (Join-Path $repo 'turbo.lock')).Hash -ne $hashes['turbo.lock']) { Fail 'repair changed turbo.lock' }
    $hashes['.turbo\tools\manifest.json'] = (Get-FileHash -LiteralPath (Join-Path $repo '.turbo\tools\manifest.json')).Hash
    $bin = Assert-Build $repo 'strict' 'filtered'
  }

  Write-Output "Standalone Windows smoke passed for turbo $Version with managed Node.js $nodeVersion and pnpm 10.0.0 (x64)."
} finally {
  $env:Path = $previousPath
  if ($mirror -and -not $mirror.HasExited) { Stop-Process -Id $mirror.Id -Force }
  if (Test-Path -LiteralPath $work) { Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue }
}
