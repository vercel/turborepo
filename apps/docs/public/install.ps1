[CmdletBinding()]
param(
  [Alias('Version')]
  [string]$TurboInstallerVersion,
  [Alias('InstallDirectory')]
  [string]$TurboInstallerInstallDirectory,
  [Alias('NoModifyPath')]
  [switch]$TurboInstallerNoModifyPath
)

$installerArguments = @{
  Version = if ($PSBoundParameters.ContainsKey('TurboInstallerVersion')) { $TurboInstallerVersion } else { $env:TURBO_VERSION }
  InstallDirectory = if ($PSBoundParameters.ContainsKey('TurboInstallerInstallDirectory')) { $TurboInstallerInstallDirectory } else { $env:TURBO_INSTALL_DIR }
  NoModifyPath = $PSBoundParameters.ContainsKey('TurboInstallerNoModifyPath') -and $TurboInstallerNoModifyPath
}

function Invoke-TurboInstaller {
  [CmdletBinding()]
  param(
    [string]$Version,
    [string]$InstallDirectory,
    [switch]$NoModifyPath
  )

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

try {
  [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
} catch {
  # Newer PowerShell versions do not require a ServicePointManager override.
}

function Get-TurboVersion {
  param([string]$RequestedVersion)

  if ([string]::IsNullOrWhiteSpace($RequestedVersion) -or $RequestedVersion -eq 'latest') {
    $response = Invoke-WebRequest `
      -Uri 'https://api.github.com/repos/vercel/turborepo/releases/latest' `
      -Headers @{ Accept = 'application/vnd.github+json'; 'X-GitHub-Api-Version' = '2022-11-28' } `
      -UseBasicParsing `
      -ErrorAction Stop
    $release = $response.Content | ConvertFrom-Json -ErrorAction Stop
    $RequestedVersion = [string]$release.tag_name
  }

  if ($RequestedVersion.StartsWith('v', [StringComparison]::OrdinalIgnoreCase)) {
    $RequestedVersion = $RequestedVersion.Substring(1)
  }
  if ($RequestedVersion -notmatch '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$') {
    throw "Invalid Turborepo release version: $RequestedVersion"
  }
  return $RequestedVersion
}

function Add-TurboToUserPath {
  param([string]$Directory)

  $normalizedDirectory = [IO.Path]::GetFullPath($Directory).TrimEnd([char[]]@('\', '/'))
  if (-not [string]::IsNullOrWhiteSpace($env:GITHUB_PATH)) {
    $normalizedGitHubDirectory = [IO.Path]::GetFullPath($Directory).TrimEnd([IO.Path]::DirectorySeparatorChar)
    $githubPathEntries = @()
    if (Test-Path -LiteralPath $env:GITHUB_PATH) {
      $githubPathEntries = [IO.File]::ReadAllLines($env:GITHUB_PATH)
    }
    $alreadyInGitHubPath = $false
    foreach ($entry in $githubPathEntries) {
      if ([string]::Equals($entry.TrimEnd([IO.Path]::DirectorySeparatorChar), $normalizedGitHubDirectory, [StringComparison]::OrdinalIgnoreCase)) {
        $alreadyInGitHubPath = $true
        break
      }
    }
    if (-not $alreadyInGitHubPath) {
      [IO.File]::AppendAllText($env:GITHUB_PATH, $Directory + [Environment]::NewLine, [Text.UTF8Encoding]::new($false))
    }

    $githubProcessEntries = @($env:Path -split ';' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    $alreadyInGitHubProcessPath = $false
    foreach ($entry in $githubProcessEntries) {
      if ([string]::Equals($entry.TrimEnd([IO.Path]::DirectorySeparatorChar), $normalizedGitHubDirectory, [StringComparison]::OrdinalIgnoreCase)) {
        $alreadyInGitHubProcessPath = $true
        break
      }
    }
    if (-not $alreadyInGitHubProcessPath) {
      $env:Path = if ([string]::IsNullOrWhiteSpace($env:Path)) { $Directory } else { "$Directory;$env:Path" }
    }
    return
  }

  $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
  $userEntries = @($userPath -split ';' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
  $alreadyInUserPath = $false
  foreach ($entry in $userEntries) {
    if ([string]::Equals($entry.TrimEnd([char[]]@('\', '/')), $normalizedDirectory, [StringComparison]::OrdinalIgnoreCase)) {
      $alreadyInUserPath = $true
      break
    }
  }
  if (-not $alreadyInUserPath) {
    $newUserPath = if ([string]::IsNullOrWhiteSpace($userPath)) { $Directory } else { "$userPath;$Directory" }
    [Environment]::SetEnvironmentVariable('Path', $newUserPath, 'User')
  }

  $processEntries = @($env:Path -split ';' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
  $alreadyInProcessPath = $false
  foreach ($entry in $processEntries) {
    if ([string]::Equals($entry.TrimEnd([char[]]@('\', '/')), $normalizedDirectory, [StringComparison]::OrdinalIgnoreCase)) {
      $alreadyInProcessPath = $true
      break
    }
  }
  if (-not $alreadyInProcessPath) {
    $env:Path = if ([string]::IsNullOrWhiteSpace($env:Path)) { $Directory } else { "$Directory;$env:Path" }
  }
}

if ([string]::IsNullOrWhiteSpace($InstallDirectory)) {
  if ([string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
    throw 'LOCALAPPDATA is not set; provide -InstallDirectory or TURBO_INSTALL_DIR.'
  }
  $InstallDirectory = Join-Path $env:LOCALAPPDATA 'Programs\Turborepo\bin'
}
$InstallDirectory = [IO.Path]::GetFullPath($InstallDirectory)
if ($InstallDirectory.IndexOfAny([char[]]@(';', "`r", "`n")) -ge 0) {
  throw 'InstallDirectory must not contain a semicolon or line break.'
}
$destination = Join-Path $InstallDirectory 'turbo.exe'
$existingTurbo = Get-Command -Name 'turbo' -All -ErrorAction SilentlyContinue |
  Where-Object { $_.CommandType -in @('Application', 'ExternalScript', 'Script') } |
  Select-Object -First 1
if ($null -ne $existingTurbo) {
  $existingTurboPath = $existingTurbo.Source
  if ([string]::IsNullOrWhiteSpace($existingTurboPath)) {
    $existingTurboPath = $existingTurbo.Path
  }
  if ([string]::IsNullOrWhiteSpace($existingTurboPath)) {
    $existingTurboPath = $existingTurbo.Definition
  }
  if (-not [string]::Equals([IO.Path]::GetFullPath($existingTurboPath), $destination, [StringComparison]::OrdinalIgnoreCase)) {
    throw ("Found an existing turbo on PATH at {0}.`nUninstall it using the tool that installed it, then rerun:`n  irm https://turborepo.dev/install.ps1 | iex" -f $existingTurboPath)
  }
}

function Test-TurboDestination {
  $item = Get-Item -LiteralPath $destination -Force -ErrorAction SilentlyContinue
  if ($null -eq $item) {
    return $false
  }
  if ($item -isnot [IO.FileInfo] -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
    throw "$destination is not a regular file; it was left untouched. Remove it yourself or choose another install directory."
  }
  return $true
}
$null = Test-TurboDestination

$architecture = $env:PROCESSOR_ARCHITECTURE
if (-not [string]::IsNullOrWhiteSpace($env:PROCESSOR_ARCHITEW6432)) {
  $architecture = $env:PROCESSOR_ARCHITEW6432
}
if ($architecture -notin @('AMD64', 'x86_64')) {
  throw "Unsupported Windows architecture: $architecture. The standalone installer currently supports Windows x64 only."
}

$tarCommand = Get-Command tar.exe -ErrorAction SilentlyContinue
if ($null -eq $tarCommand) {
  throw 'tar.exe is required to extract the standalone archive. Use a supported Windows 10 or Windows 11 installation.'
}

$workDirectory = Join-Path ([IO.Path]::GetTempPath()) ("turbo-install-" + [Guid]::NewGuid().ToString('N'))
$stagedBinary = $null
$backupBinary = $null
try {
  [IO.Directory]::CreateDirectory($workDirectory) | Out-Null
  $Version = Get-TurboVersion -RequestedVersion $Version
  $target = 'x86_64-pc-windows-msvc'
  $archiveName = "turbo-$Version-$target.tar.gz"
  $releaseUrl = "https://github.com/vercel/turborepo/releases/download/v$Version"
  $manifestPath = Join-Path $workDirectory 'SHA256SUMS'
  $archivePath = Join-Path $workDirectory $archiveName

  Invoke-WebRequest -Uri "$releaseUrl/SHA256SUMS" -OutFile $manifestPath -UseBasicParsing -ErrorAction Stop | Out-Null
  Invoke-WebRequest -Uri "$releaseUrl/$archiveName" -OutFile $archivePath -UseBasicParsing -ErrorAction Stop | Out-Null

  $expectedDigests = @()
  foreach ($line in [IO.File]::ReadAllLines($manifestPath)) {
    if ($line -cmatch '^([0-9A-Fa-f]{64})  ([^\r\n]+)$' -and $Matches[2] -ceq $archiveName) {
      $expectedDigests += $Matches[1]
    }
  }
  if ($expectedDigests.Count -ne 1) {
    throw "Checksum manifest must contain exactly one SHA-256 entry for $archiveName."
  }
  $actualDigest = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash
  if (-not [string]::Equals($actualDigest, $expectedDigests[0], [StringComparison]::OrdinalIgnoreCase)) {
    throw "SHA-256 verification failed for $archiveName."
  }

  $members = @(& $tarCommand.Source -tzf $archivePath 2>&1)
  if ($LASTEXITCODE -ne 0) {
    throw "Could not inspect $archiveName."
  }
  $members = @($members | ForEach-Object { $_.ToString().Trim() } | Where-Object { $_ -ne '' })
  if ($members.Count -ne 1 -or $members[0] -cne 'turbo.exe') {
    throw "Unexpected archive contents in $archiveName; expected only turbo.exe."
  }

  $extractDirectory = Join-Path $workDirectory 'extracted'
  [IO.Directory]::CreateDirectory($extractDirectory) | Out-Null
  & $tarCommand.Source -xzf $archivePath -C $extractDirectory
  if ($LASTEXITCODE -ne 0) {
    throw "Could not extract $archiveName."
  }
  $extractedBinary = Join-Path $extractDirectory 'turbo.exe'
  if (-not (Test-Path -LiteralPath $extractedBinary -PathType Leaf)) {
    throw 'Archive did not contain a regular turbo.exe file.'
  }
  $attributes = [IO.File]::GetAttributes($extractedBinary)
  if (($attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
    throw 'Archive turbo.exe must not be a symbolic link or reparse point.'
  }

  [IO.Directory]::CreateDirectory($InstallDirectory) | Out-Null
  $null = Test-TurboDestination
  $stagedBinary = Join-Path $InstallDirectory ('.turbo.exe.install.' + [Guid]::NewGuid().ToString('N'))
  [IO.File]::Copy($extractedBinary, $stagedBinary, $false)
  $replaced = Test-TurboDestination
  if ($replaced) {
    $backupBinary = Join-Path $InstallDirectory ('.turbo.exe.backup.' + [Guid]::NewGuid().ToString('N'))
    [IO.File]::Replace($stagedBinary, $destination, $backupBinary)
  } else {
    [IO.File]::Move($stagedBinary, $destination)
  }
  $stagedBinary = $null

  if (-not $NoModifyPath) {
    try {
      Add-TurboToUserPath -Directory $InstallDirectory
      if (-not [string]::IsNullOrWhiteSpace($env:GITHUB_PATH)) {
        Write-Host "Added $InstallDirectory to the GitHub Actions PATH."
      } else {
        Write-Host "Added $InstallDirectory to the user PATH. Open a new terminal for it to take effect there."
      }
    } catch {
      Write-Warning "turbo was installed, but PATH could not be updated. Add $InstallDirectory to PATH manually."
    }
  } else {
    Write-Host "PATH was not changed. Add $InstallDirectory to PATH to run turbo by name."
  }
  $action = if ($replaced) { "Upgraded" } else { "Installed" }
  Write-Host "$action turbo $Version at $destination"
} finally {
  if ($null -ne $stagedBinary -and (Test-Path -LiteralPath $stagedBinary)) {
    Remove-Item -LiteralPath $stagedBinary -Force
  }
  if ($null -ne $backupBinary -and (Test-Path -LiteralPath $backupBinary)) {
    Remove-Item -LiteralPath $backupBinary -Force
  }
  if (Test-Path -LiteralPath $workDirectory) {
    Remove-Item -LiteralPath $workDirectory -Recurse -Force
  }
}
}

Invoke-TurboInstaller @installerArguments
