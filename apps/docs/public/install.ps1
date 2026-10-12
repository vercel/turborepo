<#
.SYNOPSIS
Installs the standalone Turborepo CLI without Node.js or a JavaScript package manager.

.DESCRIPTION
By default, the installer uses the latest stable release, installs turbo.exe to
%LOCALAPPDATA%\Programs\Turborepo\bin, and adds that directory to the user PATH.
When the installed turbo supports it, the installer also adds the turbo shell
hook to the PowerShell profile ($PROFILE.CurrentUserAllHosts).

Each option has an environment variable for use with `irm ... | iex`:
  -Version VERSION            TURBO_VERSION
  -InstallDirectory DIRECTORY TURBO_INSTALL_DIR
  -NoModifyPath               TURBO_NO_MODIFY_PATH=1  Change neither PATH nor the profile.
  -NoShellHook                TURBO_NO_SHELL_HOOK=1   Do not add the shell hook.
  -Uninstall                  TURBO_UNINSTALL=1       Remove only what the installer
                                                      added for the install directory.
#>
[CmdletBinding()]
param(
  [Alias('Version')]
  [string]$TurboInstallerVersion,
  [Alias('InstallDirectory')]
  [string]$TurboInstallerInstallDirectory,
  [Alias('NoModifyPath')]
  [switch]$TurboInstallerNoModifyPath,
  [Alias('NoShellHook')]
  [switch]$TurboInstallerNoShellHook,
  [Alias('Uninstall')]
  [switch]$TurboInstallerUninstall
)

$installerArguments = @{
  Version = if ($PSBoundParameters.ContainsKey('TurboInstallerVersion')) { $TurboInstallerVersion } else { $env:TURBO_VERSION }
  InstallDirectory = if ($PSBoundParameters.ContainsKey('TurboInstallerInstallDirectory')) { $TurboInstallerInstallDirectory } else { $env:TURBO_INSTALL_DIR }
  NoModifyPath = ($PSBoundParameters.ContainsKey('TurboInstallerNoModifyPath') -and $TurboInstallerNoModifyPath) -or $env:TURBO_NO_MODIFY_PATH -eq '1'
  NoShellHook = ($PSBoundParameters.ContainsKey('TurboInstallerNoShellHook') -and $TurboInstallerNoShellHook) -or $env:TURBO_NO_SHELL_HOOK -eq '1'
  Uninstall = ($PSBoundParameters.ContainsKey('TurboInstallerUninstall') -and $TurboInstallerUninstall) -or $env:TURBO_UNINSTALL -eq '1'
}

function Invoke-TurboInstaller {
  [CmdletBinding()]
  param(
    [string]$Version,
    [string]$InstallDirectory,
    [switch]$NoModifyPath,
    [switch]$NoShellHook,
    [switch]$Uninstall
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

$hookMarker = '# Turborepo standalone installer shell hook'

# Keeps profile bytes intact: files without a byte order mark round-trip
# through Latin-1, and lines the installer writes are stored there as UTF-8.
function Read-TurboProfile {
  param([string]$Path)

  if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
    return [pscustomobject]@{ Encoding = [Text.UTF8Encoding]::new($true); Text = '' }
  }
  $bytes = [IO.File]::ReadAllBytes($Path)
  $encoding = [Text.Encoding]::GetEncoding(28591)
  $offset = 0
  if ($bytes.Length -ge 3 -and $bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF) {
    $encoding = [Text.UTF8Encoding]::new($true)
    $offset = 3
  } elseif ($bytes.Length -ge 2 -and (($bytes[0] -eq 0xFF -and $bytes[1] -eq 0xFE) -or ($bytes[0] -eq 0xFE -and $bytes[1] -eq 0xFF))) {
    $encoding = [Text.UnicodeEncoding]::new($bytes[0] -eq 0xFE, $true)
    $offset = 2
  }
  return [pscustomobject]@{ Encoding = $encoding; Text = $encoding.GetString($bytes, $offset, $bytes.Length - $offset) }
}

function ConvertTo-TurboProfileLine {
  param($Content, [string]$Line)

  if ($Content.Encoding.CodePage -eq 28591) {
    return $Content.Encoding.GetString([Text.Encoding]::UTF8.GetBytes($Line))
  }
  return $Line
}

function Get-TurboHookLine {
  $quote = "['$([char]0x2018)$([char]0x2019)$([char]0x201A)$([char]0x201B)]"
  return "Invoke-Expression (& '$($destination -replace $quote, '$0$0')' hook powershell | Out-String) $hookMarker"
}

# Owns one marked line per profile: replace it in place, dropping duplicates,
# or append it.
function Set-TurboProfileHook {
  param([string]$Path)

  $content = Read-TurboProfile -Path $Path
  $line = ConvertTo-TurboProfileLine -Content $content -Line (Get-TurboHookLine)
  $segments = @([regex]::Split($content.Text, '(?<=\n)') | Where-Object { $_ -ne '' })
  $marked = @($segments | Where-Object { $_.Contains($hookMarker) })
  if ($marked.Count -eq 1 -and $marked[0].TrimEnd([char[]]@("`r", "`n")) -ceq $line) {
    return
  }
  if ($marked.Count -eq 0) {
    $newline = if ($content.Text.Contains("`n") -and -not $content.Text.Contains("`r`n")) { "`n" } else { "`r`n" }
    $text = if ($content.Text.Length -eq 0) { $line + $newline } else { $content.Text + $newline + $line + $newline }
  } else {
    $replaced = $false
    $text = -join @(foreach ($segment in $segments) {
      if (-not $segment.Contains($hookMarker)) {
        $segment
      } elseif (-not $replaced) {
        $replaced = $true
        $line + $segment.Substring($segment.TrimEnd([char[]]@("`r", "`n")).Length)
      }
    })
  }
  [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($Path)) | Out-Null
  [IO.File]::WriteAllText($Path, $text, $content.Encoding)
}

# New sessions ignore the Process scope, so check the scopes they will use.
function Get-TurboProfileExecutionPolicy {
  foreach ($scope in @('MachinePolicy', 'UserPolicy', 'CurrentUser', 'LocalMachine')) {
    $policy = [string](Get-ExecutionPolicy -Scope $scope)
    if ($policy -ne 'Undefined') {
      return $policy
    }
  }
  if ($PSVersionTable.PSEdition -eq 'Core') { return 'RemoteSigned' }
  return 'Restricted'
}

function Add-TurboShellHook {
  $profilePath = $PROFILE.CurrentUserAllHosts
  if ([string]::IsNullOrWhiteSpace($profilePath)) {
    return
  }
  # Older releases have no hook command.
  try {
    & $destination hook powershell *> $null
    if ($LASTEXITCODE -ne 0) {
      return
    }
  } catch {
    return
  }
  $policy = Get-TurboProfileExecutionPolicy
  if ($policy -in @('Restricted', 'AllSigned')) {
    Write-Host "Skipped the turbo shell hook because the PowerShell execution policy ($policy) does not run profile scripts."
    return
  }
  try {
    Set-TurboProfileHook -Path $profilePath
    Write-Host "Added the turbo shell hook to $profilePath. Open a new terminal to use it."
  } catch {
    Write-Warning "turbo was installed, but the shell hook could not be added to $profilePath."
  }
}

# Removes this installation's hook line, with the line break the installer
# wrote before it, so the profile returns to its prior bytes.
function Remove-TurboProfileHook {
  param([string]$Path)

  if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
    return
  }
  $content = Read-TurboProfile -Path $Path
  $line = ConvertTo-TurboProfileLine -Content $content -Line (Get-TurboHookLine)
  $segments = [Collections.Generic.List[string]]@([regex]::Split($content.Text, '(?<=\n)') | Where-Object { $_ -ne '' })
  $removed = $false
  for ($index = $segments.Count - 1; $index -ge 0; $index--) {
    if ($segments[$index].TrimEnd([char[]]@("`r", "`n")) -cne $line) {
      continue
    }
    $removed = $true
    $segments.RemoveAt($index)
    if ($index -gt 0 -and $segments[$index - 1].Trim([char[]]@("`r", "`n")).Length -eq 0) {
      $index--
      $segments.RemoveAt($index)
    } elseif ($index -gt 0 -and $index -eq $segments.Count) {
      # The installer ended this line when the profile had no final newline.
      $segments[$index - 1] = $segments[$index - 1].TrimEnd([char[]]@("`r", "`n"))
    }
  }
  if ($removed) {
    if ($segments.Count -eq 0) {
      Remove-Item -LiteralPath $Path -Force
    } else {
      [IO.File]::WriteAllText($Path, (-join $segments), $content.Encoding)
    }
    Write-Host "Removed the turbo shell hook from $Path"
  }
  if ((Test-Path -LiteralPath $Path) -and (Read-TurboProfile -Path $Path).Text.Contains($hookMarker)) {
    Write-Host "Left lines in $Path that are marked by the installer but do not match this installation."
  }
}

function Remove-TurboFromUserPath {
  param([string]$Directory)

  $normalizedDirectory = $Directory.TrimEnd([char[]]@('\', '/'))
  $isInstallDirectory = { param($entry) [string]::Equals($entry.TrimEnd([char[]]@('\', '/')), $normalizedDirectory, [StringComparison]::OrdinalIgnoreCase) }
  $env:Path = @($env:Path -split ';' | Where-Object { -not (& $isInstallDirectory $_) }) -join ';'
  $entries = @([Environment]::GetEnvironmentVariable('Path', 'User') -split ';')
  $kept = @($entries | Where-Object { -not (& $isInstallDirectory $_) })
  if ($kept.Count -eq $entries.Count) {
    return $false
  }
  [Environment]::SetEnvironmentVariable('Path', ($kept -join ';'), 'User')
  return $true
}

# Removes only what the installer owns: a regular turbo.exe at the destination
# (the same rule upgrades use), the install directory and its user PATH entry
# once nothing else is in it, and this installation's hook line in the
# Windows PowerShell and PowerShell 7 profiles.
function Uninstall-Turbo {
  $item = Get-Item -LiteralPath $destination -Force -ErrorAction SilentlyContinue
  if ($null -eq $item) {
    Write-Host "No turbo.exe found at $destination"
  } elseif ($item -isnot [IO.FileInfo] -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
    Write-Host "Left $destination because it is not a regular file."
  } else {
    try {
      Remove-Item -LiteralPath $destination -Force
    } catch {
      throw "Could not remove $destination. Close any running turbo processes, then try again."
    }
    Write-Host "Removed $destination"
  }

  if (@(Get-ChildItem -LiteralPath $InstallDirectory -Force -ErrorAction SilentlyContinue).Count -eq 0) {
    $directories = @($InstallDirectory)
    if (-not [string]::IsNullOrWhiteSpace($env:LOCALAPPDATA) -and [string]::Equals($InstallDirectory, [IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA 'Programs\Turborepo\bin')), [StringComparison]::OrdinalIgnoreCase)) {
      $directories += Split-Path -Parent $InstallDirectory
    }
    foreach ($directory in $directories) {
      if ((Test-Path -LiteralPath $directory -PathType Container) -and @(Get-ChildItem -LiteralPath $directory -Force).Count -eq 0) {
        Remove-Item -LiteralPath $directory -Force
      }
    }
    if (Remove-TurboFromUserPath -Directory $InstallDirectory) {
      Write-Host "Removed $InstallDirectory from the user PATH."
    }
  } elseif (@([Environment]::GetEnvironmentVariable('Path', 'User') -split ';') -contains $InstallDirectory) {
    Write-Host "Left $InstallDirectory on the user PATH because it still contains other files."
  }

  $profilePath = $PROFILE.CurrentUserAllHosts
  if (-not [string]::IsNullOrWhiteSpace($profilePath)) {
    $documents = Split-Path -Parent (Split-Path -Parent $profilePath)
    $profilePaths = @($profilePath) + @(@('WindowsPowerShell', 'PowerShell') | ForEach-Object { Join-Path $documents "$_\profile.ps1" } | Where-Object { $_ -ne $profilePath })
    foreach ($path in $profilePaths) {
      Remove-TurboProfileHook -Path $path
    }
  }

  $otherTurbo = Get-Command -Name 'turbo' -All -ErrorAction SilentlyContinue |
    Where-Object { $_.CommandType -in @('Application', 'ExternalScript', 'Script') -and $_.Source -ne $destination } |
    Select-Object -First 1
  if ($null -ne $otherTurbo) {
    Write-Host "Left $($otherTurbo.Source) because the installer did not install it."
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
if ($Uninstall) {
  Uninstall-Turbo
  return
}
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
# x64 PowerShell under emulation on arm64 reports AMD64 above, so also ask
# Windows for the host processor (Win32_Processor.Architecture 12 is ARM64).
$hostProcessor = $null
try {
  $hostProcessor = @(Get-CimInstance -ClassName Win32_Processor -ErrorAction Stop)[0].Architecture
} catch {
  # Without CIM, the process architecture above is all there is to go on.
}
if ($architecture -eq 'ARM64' -or $hostProcessor -eq 12) {
  throw "Standalone turbo is not available for Windows arm64 yet, and this installer will not install the x64 build to run under emulation.`nUse turbo in WSL, or install it with your package manager (for example, npm install turbo --save-dev)."
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
  if (-not $NoModifyPath -and -not $NoShellHook -and [string]::IsNullOrWhiteSpace($env:GITHUB_PATH)) {
    Add-TurboShellHook
  }
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
