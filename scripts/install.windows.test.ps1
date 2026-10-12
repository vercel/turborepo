$ErrorActionPreference = 'Stop'
$installerPath = Join-Path $PSScriptRoot '..\apps\docs\public\install.ps1'
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ("turbo-installer-test-" + [Guid]::NewGuid().ToString('N'))
$previousGitHubPath = $env:GITHUB_PATH
$previousTurboVersion = $env:TURBO_VERSION
$previousTurboInstallDirectory = $env:TURBO_INSTALL_DIR
$previousPath = $env:Path
$previousNoModifyPath = $env:TURBO_NO_MODIFY_PATH
$previousNoShellHook = $env:TURBO_NO_SHELL_HOOK
$previousUninstall = $env:TURBO_UNINSTALL
$previousLocalAppData = $env:LOCALAPPDATA
# Shell hook tests let the installer edit the user PATH; restore its exact value.
$environmentKey = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
$previousUserPath = $environmentKey.GetValue('Path', $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
$previousUserPathKind = if ($null -ne $previousUserPath) { $environmentKey.GetValueKind('Path') } else { $null }
$hookMarker = '# Turborepo standalone installer shell hook'
$script:RequestCount = 0
$script:ChecksumFixture = $null
$script:ArchiveFixture = $null
$script:ArchiveName = $null
$script:ExecutionPolicy = 'RemoteSigned'

function Assert-True {
  param([bool]$Condition, [string]$Message)
  if (-not $Condition) {
    throw $Message
  }
}

function Get-ExecutionPolicy {
  param([string]$Scope)
  if ($Scope -eq 'CurrentUser') {
    return $script:ExecutionPolicy
  }
  return 'Undefined'
}

function New-FixtureExecutable {
  param([string]$Name, [string]$Body)
  $path = Join-Path $testRoot "$Name.exe"
  Add-Type -TypeDefinition "public static class $Name { public static int Main(string[] args) { $Body } }" -OutputAssembly $path -OutputType ConsoleApplication
  return $path
}

function Set-ArchiveFixture {
  param([string]$Binary)
  $directory = Join-Path $testRoot ('archive-' + [Guid]::NewGuid().ToString('N'))
  [IO.Directory]::CreateDirectory($directory) | Out-Null
  Copy-Item -LiteralPath $Binary -Destination (Join-Path $directory 'turbo.exe')
  $script:ArchiveFixture = Join-Path $testRoot ([Guid]::NewGuid().ToString('N') + '.tar.gz')
  & $tarCommand.Source -czf $script:ArchiveFixture -C $directory 'turbo.exe'
  Assert-True ($LASTEXITCODE -eq 0) 'Could not create the test archive.'
  $digest = (Get-FileHash -LiteralPath $script:ArchiveFixture -Algorithm SHA256).Hash.ToLowerInvariant()
  [IO.File]::WriteAllText($script:ChecksumFixture, "$digest  $script:ArchiveName`n")
}

function Invoke-Installer {
  param([hashtable]$Arguments)
  # Out-String wraps host output at the console width.
  return (. $installerPath @Arguments 6>&1 | ForEach-Object { [string]$_ }) -join "`n"
}

function Test-UserPathContains {
  param([string]$Directory)
  return @([Environment]::GetEnvironmentVariable('Path', 'User') -split ';') -contains $Directory
}

function Invoke-WebRequest {
  [CmdletBinding()]
  param(
    [string]$Uri,
    [string]$OutFile,
    [switch]$UseBasicParsing,
    [hashtable]$Headers
  )

  $script:RequestCount++
  if ($Uri -like '*/repos/vercel/turborepo/releases/latest') {
    return [pscustomobject]@{ Content = '{"tag_name":"v2.11.5"}' }
  }
  if ($Uri -like '*/SHA256SUMS' -and $OutFile) {
    Copy-Item -LiteralPath $script:ChecksumFixture -Destination $OutFile
    return
  }
  if ($Uri -like "*/$script:ArchiveName" -and $OutFile) {
    Copy-Item -LiteralPath $script:ArchiveFixture -Destination $OutFile
    return
  }
  throw "Unexpected installer request: $Uri"
}

try {
  [IO.Directory]::CreateDirectory($testRoot) | Out-Null
  $githubPathFixture = Join-Path $testRoot 'github-path'
  [IO.File]::WriteAllText($githubPathFixture, '')
  $env:GITHUB_PATH = $githubPathFixture
  $expectedInstallDirectory = Join-Path $testRoot 'install dir'
  $existingTurboCommand = Get-Command -Name 'turbo' -All -ErrorAction SilentlyContinue |
    Where-Object { $_.CommandType -in @('Application', 'ExternalScript', 'Script') } |
    Select-Object -First 1
  if ($null -ne $existingTurboCommand) {
    $existingTurboPath = $existingTurboCommand.Source
    if ([string]::IsNullOrWhiteSpace($existingTurboPath)) {
      $existingTurboPath = $existingTurboCommand.Path
    }
    if ([string]::IsNullOrWhiteSpace($existingTurboPath)) {
      $existingTurboPath = $existingTurboCommand.Definition
    }
    $realCollisionInstallDirectory = Join-Path $testRoot 'real-path-collision-install'
    $failed = $false
    try {
      . $installerPath -Version '2.11.5' -InstallDirectory $realCollisionInstallDirectory -NoModifyPath
    } catch {
      $failureMessage = $_.Exception.Message
      $failed = $failureMessage -match 'Found an existing turbo on PATH at'
      Assert-True ($failureMessage.Contains($existingTurboPath)) 'PATH collision error omitted the real executable path.'
      Assert-True ($failureMessage -match 'Uninstall it using the tool that installed it') 'Installer omitted the retry instruction.'
      Assert-True ($failureMessage.Contains('irm https://turborepo.dev/install.ps1 | iex')) 'Installer omitted the rerun command.'
    }
    Assert-True $failed 'Installer did not reject the real existing turbo command on PATH.'
    Assert-True ($script:RequestCount -eq 0) 'Installer downloaded files before refusing the real PATH collision.'
    Assert-True (-not (Test-Path -LiteralPath $realCollisionInstallDirectory)) 'Installer wrote files before refusing the real PATH collision.'
    Assert-True ([IO.File]::ReadAllLines($githubPathFixture).Count -eq 0) 'Installer modified GITHUB_PATH before refusing the real PATH collision.'
    Write-Output "PASS: refused the existing turbo command at $existingTurboPath without executing it."
  }

  $systemPath = @(
    (Join-Path $env:SystemRoot 'System32'),
    (Join-Path $env:SystemRoot 'System32\Wbem'),
    (Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0')
  ) -join [IO.Path]::PathSeparator
  $env:Path = $systemPath
  $architecture = $env:PROCESSOR_ARCHITECTURE
  if (-not [string]::IsNullOrWhiteSpace($env:PROCESSOR_ARCHITEW6432)) {
    $architecture = $env:PROCESSOR_ARCHITEW6432
  }

  if ($architecture -notin @('AMD64', 'x86_64')) {
    $failed = $false
    try {
      . $installerPath -Version '2.11.5' -InstallDirectory $expectedInstallDirectory -NoModifyPath
    } catch {
      $failed = $_.Exception.Message -match 'Unsupported Windows architecture'
    }
    Assert-True $failed "Installer did not reject unsupported Windows architecture $architecture."
    Assert-True ($script:RequestCount -eq 0) 'Installer made a network request before rejecting the architecture.'
    Assert-True (-not (Test-Path -LiteralPath $expectedInstallDirectory)) 'Installer changed files before rejecting the architecture.'
    Write-Output "PASS: rejected unsupported Windows architecture $architecture before download or filesystem changes."
    $env:PROCESSOR_ARCHITECTURE = 'AMD64'
    Remove-Item Env:PROCESSOR_ARCHITEW6432 -ErrorAction SilentlyContinue
    Write-Output 'Continuing with simulated AMD64 metadata for fixture-only installer tests; no downloaded executable is run.'
  }

  $unsafeInstallDirectory = Join-Path $testRoot 'unsafe;path'
  $failed = $false
  try {
    . $installerPath -Version '2.11.5' -InstallDirectory $unsafeInstallDirectory -NoModifyPath
  } catch {
    $failed = $_.Exception.Message -match 'must not contain a semicolon or line break'
  }
  Assert-True $failed 'Installer accepted an install directory that would inject another PATH entry.'
  Assert-True (-not (Test-Path -LiteralPath $unsafeInstallDirectory)) 'Installer created an unsafe install directory.'
  Assert-True ($script:RequestCount -eq 0) 'Installer made a network request before rejecting an unsafe install directory.'

  $tarCommand = Get-Command tar.exe -ErrorAction Stop
  $fixtureDirectory = Join-Path $testRoot 'fixture'
  [IO.Directory]::CreateDirectory($fixtureDirectory) | Out-Null
  [IO.File]::WriteAllText((Join-Path $fixtureDirectory 'turbo.exe'), 'standalone turbo fixture')
  $script:ArchiveName = 'turbo-2.11.5-x86_64-pc-windows-msvc.tar.gz'
  $script:ArchiveFixture = Join-Path $testRoot $script:ArchiveName
  Push-Location $fixtureDirectory
  try {
    & $tarCommand.Source -czf $script:ArchiveFixture 'turbo.exe'
    Assert-True ($LASTEXITCODE -eq 0) 'Could not create the test archive.'
  } finally {
    Pop-Location
  }

  $digest = (Get-FileHash -LiteralPath $script:ArchiveFixture -Algorithm SHA256).Hash.ToLowerInvariant()
  $script:ChecksumFixture = Join-Path $testRoot 'SHA256SUMS'
  [IO.File]::WriteAllText($script:ChecksumFixture, "$digest  $script:ArchiveName`n")
  $script:RequestCount = 0

  $env:TURBO_VERSION = 'latest'
  $env:TURBO_INSTALL_DIR = $expectedInstallDirectory
  $Version = '9.9.9'
  $InstallDirectory = Join-Path $testRoot 'ambient-install-directory'
  $NoModifyPath = $true
  Invoke-Expression ([IO.File]::ReadAllText($installerPath))
  $installedBinary = Join-Path $expectedInstallDirectory 'turbo.exe'
  Assert-True (Test-Path -LiteralPath $installedBinary -PathType Leaf) 'Installer did not install turbo.exe.'
  Assert-True ([IO.File]::ReadAllText($installedBinary) -eq 'standalone turbo fixture') 'Installed binary contents changed.'
  Assert-True ($script:RequestCount -eq 3) 'Expected latest-version, checksum, and archive requests.'
  Assert-True ([IO.File]::ReadAllLines($githubPathFixture) -contains $expectedInstallDirectory) 'Installer did not add its directory to GITHUB_PATH.'
  Assert-True (($env:Path -split ';') -contains $expectedInstallDirectory) 'Installer did not add its directory to the current PATH.'
  Assert-True (-not (Test-Path -LiteralPath $InstallDirectory)) 'Piped execution used an ambient install-directory variable.'
  $env:Path = $systemPath

  $badInstallDirectory = Join-Path $testRoot 'bad-digest-install'
  [IO.File]::WriteAllText($script:ChecksumFixture, (('0' * 64) + "  $script:ArchiveName`n"))
  $failed = $false
  $failureMessage = ''
  try {
    . $installerPath -Version '2.11.5' -InstallDirectory $badInstallDirectory -NoModifyPath
  } catch {
    $failureMessage = $_.Exception.Message
    $failed = $failureMessage -match 'SHA-256 verification failed'
  }
  Assert-True $failed "Installer did not reject a mismatched digest. Received: $failureMessage"
  Assert-True (-not (Test-Path -LiteralPath $badInstallDirectory)) 'Installer created files after a digest failure.'

  [IO.File]::WriteAllText($script:ChecksumFixture, "$digest  $script:ArchiveName`n")
  $existingDirectory = Join-Path $testRoot 'existing-install'
  [IO.Directory]::CreateDirectory($existingDirectory) | Out-Null
  $existingBinary = Join-Path $existingDirectory 'turbo.exe'
  [IO.File]::WriteAllText($existingBinary, 'old binary')
  $env:Path = "$existingDirectory;$systemPath"
  $requestCountBeforeUpgrade = $script:RequestCount
  $upgradeOutput = . $installerPath -Version '2.11.5' -InstallDirectory $existingDirectory -NoModifyPath 6>&1
  Assert-True (($upgradeOutput | Out-String) -match 'Upgraded turbo 2.11.5 at') 'Installer did not announce the upgrade.'
  Assert-True ([IO.File]::ReadAllText($existingBinary) -eq 'standalone turbo fixture') 'Installer did not replace its existing binary.'
  Assert-True (@(Get-ChildItem -LiteralPath $existingDirectory -Filter '.turbo.exe.*').Count -eq 0) 'Installer left a staging or backup file behind.'
  Assert-True ($script:RequestCount -eq ($requestCountBeforeUpgrade + 2)) 'Upgrade did not download and verify the archive.'

  [IO.File]::WriteAllText($script:ChecksumFixture, (('0' * 64) + "  $script:ArchiveName`n"))
  $failed = $false
  try {
    . $installerPath -Version '2.11.5' -InstallDirectory $existingDirectory -NoModifyPath
  } catch {
    $failed = $_.Exception.Message -match 'SHA-256 verification failed'
  }
  Assert-True $failed 'Installer accepted a bad digest while upgrading.'
  Assert-True ([IO.File]::ReadAllText($existingBinary) -eq 'standalone turbo fixture') 'Failed upgrade changed the existing executable.'
  $env:Path = $systemPath

  $collisionDirectory = Join-Path $testRoot 'existing-path'
  [IO.Directory]::CreateDirectory($collisionDirectory) | Out-Null
  $collisionShim = Join-Path $collisionDirectory 'turbo.cmd'
  $collisionMarker = Join-Path $testRoot 'existing-turbo-was-run'
  [IO.File]::WriteAllText($collisionShim, "@echo off`r`necho ran > `"$collisionMarker`"`r`n")
  $env:Path = "$collisionDirectory;$env:Path"
  $requestCountBeforePathCollision = $script:RequestCount
  $failed = $false
  try {
    . $installerPath -Version '2.11.5' -InstallDirectory (Join-Path $testRoot 'path-collision-install') -NoModifyPath
  } catch {
    $failed = $_.Exception.Message -match 'Found an existing turbo on PATH at'
    Assert-True ($_.Exception.Message.Contains($collisionShim)) 'PATH collision error omitted the executable path.'
    Assert-True ($_.Exception.Message -match 'Uninstall it using the tool that installed it') 'Installer omitted the retry instruction.'
    Assert-True ($_.Exception.Message.Contains('irm https://turborepo.dev/install.ps1 | iex')) 'Installer omitted the rerun command.'
  }
  Assert-True $failed 'Installer did not refuse an existing turbo.cmd on PATH.'
  Assert-True ($script:RequestCount -eq $requestCountBeforePathCollision) 'Installer downloaded files before refusing the PATH collision.'
  Assert-True (-not (Test-Path -LiteralPath (Join-Path $testRoot 'path-collision-install'))) 'Installer wrote files before refusing the PATH collision.'
  Assert-True (-not (Test-Path -LiteralPath $collisionMarker)) 'Installer executed the pre-existing turbo command.'
  $env:Path = $previousPath

  Write-Output 'PASS: Windows installer replaces its own binary, refuses PATH shims, and preserves files on failure.'

  Remove-Item Env:GITHUB_PATH -ErrorAction SilentlyContinue
  $hookTurbo = New-FixtureExecutable -Name 'HookTurbo' -Body 'if (args.Length == 2 && args[0] == "hook" && args[1] == "powershell") { System.Console.WriteLine("$env:TURBO_HOOKED = ''powershell''"); return 0; } return 1;'
  $oldTurbo = New-FixtureExecutable -Name 'OldTurbo' -Body 'return 1;'
  $profilePath = Join-Path $testRoot 'Documents\WindowsPowerShell\profile.ps1'
  $PROFILE = [pscustomobject]@{ CurrentUserAllHosts = $profilePath }
  $hookDirectory = Join-Path $testRoot "hook o'install"
  $hookLine = "Invoke-Expression (& '$((Join-Path $hookDirectory 'turbo.exe').Replace("'", "''"))' hook powershell | Out-String) $hookMarker"
  Set-ArchiveFixture -Binary $hookTurbo
  $env:Path = $systemPath
  $hookOutput = Invoke-Installer -Arguments @{ Version = '2.11.5'; InstallDirectory = $hookDirectory }
  Assert-True ($hookOutput.Contains("Added the turbo shell hook to $profilePath")) "Installer did not report the shell hook. Received: $hookOutput"
  Assert-True (Test-UserPathContains -Directory $hookDirectory) 'Installer did not add its directory to the user PATH.'
  $profileBytes = [IO.File]::ReadAllBytes($profilePath)
  Assert-True ($profileBytes[0] -eq 0xEF -and $profileBytes[1] -eq 0xBB -and $profileBytes[2] -eq 0xBF) 'New profile is missing the UTF-8 byte order mark Windows PowerShell needs.'
  Assert-True ([IO.File]::ReadAllText($profilePath) -ceq "$hookLine`r`n") 'Installer wrote an unexpected new profile.'
  $powershell = (Get-Process -Id $PID).Path
  $hooked = & $powershell -NoLogo -NoProfile -ExecutionPolicy Bypass -Command ". '$profilePath'; `$env:TURBO_HOOKED"
  Assert-True ($hooked -eq 'powershell') "The profile line did not load the turbo hook. Received: $hooked"

  # Re-running or upgrading replaces the first marked line in place and drops duplicates.
  $staleLine = "Invoke-Expression (& 'C:\old\turbo.exe' hook powershell | Out-String) $hookMarker"
  [IO.File]::WriteAllText($profilePath, "# mine`r`n$staleLine`r`n# also mine`r`n$staleLine`r`n")
  foreach ($run in 1..2) {
    $env:Path = $systemPath
    $null = Invoke-Installer -Arguments @{ Version = '2.11.5'; InstallDirectory = $hookDirectory }
  }
  Assert-True ([IO.File]::ReadAllText($profilePath) -ceq "# mine`r`n$hookLine`r`n# also mine`r`n") 'Installer did not keep exactly one hook line in place.'
  Write-Output 'PASS: Windows installer adds the shell hook once and updates only its own line.'

  $scenarios = @(
    @{ Name = '-NoShellHook'; Arguments = @{ NoShellHook = $true }; UserPath = $true },
    @{ Name = 'TURBO_NO_SHELL_HOOK=1'; Variable = 'TURBO_NO_SHELL_HOOK'; UserPath = $true },
    @{ Name = 'an older release'; Binary = $oldTurbo; UserPath = $true },
    @{ Name = 'a Restricted execution policy'; Policy = 'Restricted'; UserPath = $true; Output = 'Skipped the turbo shell hook' },
    @{ Name = '-NoModifyPath'; Arguments = @{ NoModifyPath = $true }; UserPath = $false },
    @{ Name = 'TURBO_NO_MODIFY_PATH=1'; Variable = 'TURBO_NO_MODIFY_PATH'; UserPath = $false },
    @{ Name = 'GitHub Actions'; GitHub = $true; UserPath = $false }
  )
  for ($index = 0; $index -lt $scenarios.Count; $index++) {
    $scenario = $scenarios[$index]
    Remove-Item -LiteralPath $profilePath -Force -ErrorAction SilentlyContinue
    $directory = Join-Path $testRoot "opt-out-$index"
    $arguments = @{ Version = '2.11.5'; InstallDirectory = $directory }
    if ($scenario.Arguments) { $arguments += $scenario.Arguments }
    if ($scenario.Variable) { Set-Item -Path "Env:$($scenario.Variable)" -Value '1' }
    if ($scenario.GitHub) { $env:GITHUB_PATH = $githubPathFixture }
    if ($scenario.Policy) { $script:ExecutionPolicy = $scenario.Policy }
    Set-ArchiveFixture -Binary $(if ($scenario.Binary) { $scenario.Binary } else { $hookTurbo })
    $env:Path = $systemPath
    try {
      $output = Invoke-Installer -Arguments $arguments
    } finally {
      if ($scenario.Variable) { Remove-Item -Path "Env:$($scenario.Variable)" }
      Remove-Item Env:GITHUB_PATH -ErrorAction SilentlyContinue
      $script:ExecutionPolicy = 'RemoteSigned'
    }
    Assert-True ($output.Contains('Installed turbo 2.11.5')) "Install failed for $($scenario.Name). Received: $output"
    Assert-True (-not (Test-Path -LiteralPath $profilePath)) "Installer edited the profile for $($scenario.Name)."
    Assert-True ((Test-UserPathContains -Directory $directory) -eq $scenario.UserPath) "Installer handled the user PATH incorrectly for $($scenario.Name)."
    if ($scenario.Output) {
      Assert-True ($output.Contains($scenario.Output)) "Installer did not explain the skipped hook for $($scenario.Name). Received: $output"
    } else {
      Assert-True (-not $output.Contains('shell hook')) "Installer mentioned the shell hook for $($scenario.Name). Received: $output"
    }
  }
  Write-Output 'PASS: Windows installer skips the shell hook for each opt-out, CI, older releases, and blocked profiles.'

  Remove-Item Env:TURBO_INSTALL_DIR -ErrorAction SilentlyContinue
  $env:LOCALAPPDATA = Join-Path $testRoot 'LocalAppData'
  $defaultDirectory = Join-Path $env:LOCALAPPDATA 'Programs\Turborepo\bin'
  $editionProfile = Join-Path $testRoot 'Documents\PowerShell\profile.ps1'
  $userPathBeforeInstall = [Environment]::GetEnvironmentVariable('Path', 'User')
  $roundTrips = @(
    @{ Name = 'an existing profile without a final newline'; Profile = $profilePath; Bytes = [byte[]](@(0xEF, 0xBB, 0xBF) + [Text.Encoding]::UTF8.GetBytes("# mine`r`nSet-Alias ll Get-ChildItem")) },
    @{ Name = 'a new profile'; Profile = $profilePath; Bytes = $null },
    @{ Name = "the other edition's UTF-16 profile"; Profile = $editionProfile; Bytes = [byte[]]([Text.Encoding]::Unicode.GetPreamble() + [Text.Encoding]::Unicode.GetBytes("# seven`n")); Variable = $true }
  )
  foreach ($roundTrip in $roundTrips) {
    foreach ($path in @($profilePath, $editionProfile)) {
      Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
    }
    if ($null -ne $roundTrip.Bytes) {
      [IO.Directory]::CreateDirectory((Split-Path -Parent $roundTrip.Profile)) | Out-Null
      [IO.File]::WriteAllBytes($roundTrip.Profile, $roundTrip.Bytes)
    }
    $PROFILE = [pscustomobject]@{ CurrentUserAllHosts = $roundTrip.Profile }
    Set-ArchiveFixture -Binary $hookTurbo
    $env:Path = $systemPath
    $null = Invoke-Installer -Arguments @{ Version = '2.11.5' }
    Assert-True ([IO.File]::ReadAllText($roundTrip.Profile).Contains($hookMarker)) "Installer did not add the hook for $($roundTrip.Name)."
    Assert-True (Test-UserPathContains -Directory $defaultDirectory) "Installer did not add the default directory to the user PATH for $($roundTrip.Name)."

    # Uninstalling from Windows PowerShell also cleans the PowerShell 7 profile.
    $PROFILE = [pscustomobject]@{ CurrentUserAllHosts = $profilePath }
    $requestsBeforeUninstall = $script:RequestCount
    $arguments = @{ Uninstall = $true }
    if ($roundTrip.Variable) {
      $env:TURBO_UNINSTALL = '1'
      $arguments = @{}
    }
    try {
      $output = Invoke-Installer -Arguments $arguments
    } finally {
      Remove-Item Env:TURBO_UNINSTALL -ErrorAction SilentlyContinue
    }
    $after = if (Test-Path -LiteralPath $roundTrip.Profile) { [Convert]::ToBase64String([IO.File]::ReadAllBytes($roundTrip.Profile)) } else { $null }
    $expected = if ($null -ne $roundTrip.Bytes) { [Convert]::ToBase64String($roundTrip.Bytes) } else { $null }
    Assert-True ($after -ceq $expected) "Uninstall did not restore $($roundTrip.Name) byte for byte."
    Assert-True ([Environment]::GetEnvironmentVariable('Path', 'User') -ceq $userPathBeforeInstall) "Uninstall did not restore the user PATH for $($roundTrip.Name)."
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $env:LOCALAPPDATA 'Programs\Turborepo'))) "Uninstall left the default install directory for $($roundTrip.Name)."
    Assert-True ($script:RequestCount -eq $requestsBeforeUninstall) 'Uninstall made a network request.'
    foreach ($message in @("Removed $defaultDirectory\turbo.exe", "Removed $defaultDirectory from the user PATH.", "Removed the turbo shell hook from $($roundTrip.Profile)")) {
      Assert-True ($output.Contains($message)) "Uninstall did not report '$message'. Received: $output"
    }
    Assert-True (-not $output.Contains('Left ')) "Uninstall reported leftovers for $($roundTrip.Name). Received: $output"
  }
  Write-Output 'PASS: Windows uninstall restores PowerShell profiles byte for byte and removes its PATH entry and directory.'

  $keptDirectory = Join-Path $testRoot 'kept install'
  $otherDirectory = Join-Path $testRoot 'other-path'
  [IO.Directory]::CreateDirectory($otherDirectory) | Out-Null
  $otherShim = Join-Path $otherDirectory 'turbo.cmd'
  $otherMarker = Join-Path $testRoot 'other-turbo-was-run'
  [IO.File]::WriteAllText($otherShim, "@echo off`r`necho ran > `"$otherMarker`"`r`n")
  $unmarkedProfile = "# mine`r`nInvoke-Expression (& turbo hook powershell | Out-String)`r`n"
  $foreignProfile = "Invoke-Expression (& 'C:\elsewhere\turbo.exe' hook powershell | Out-String) $hookMarker`r`n"
  [IO.File]::WriteAllText($profilePath, $unmarkedProfile)
  [IO.File]::WriteAllText($editionProfile, $foreignProfile)
  $env:Path = $systemPath
  $null = Invoke-Installer -Arguments @{ Version = '2.11.5'; InstallDirectory = $keptDirectory }
  [IO.File]::WriteAllText((Join-Path $keptDirectory 'notes.txt'), 'not turbo')
  $directoryDestination = Join-Path $testRoot 'directory destination'
  [IO.Directory]::CreateDirectory((Join-Path $directoryDestination 'turbo.exe')) | Out-Null
  $env:Path = "$otherDirectory;$systemPath"
  $output = Invoke-Installer -Arguments @{ InstallDirectory = $keptDirectory; Uninstall = $true }
  $output += "`n" + (Invoke-Installer -Arguments @{ InstallDirectory = $directoryDestination; Uninstall = $true })
  Assert-True (-not (Test-Path -LiteralPath (Join-Path $keptDirectory 'turbo.exe'))) 'Uninstall did not remove its own turbo.exe.'
  Assert-True ([IO.File]::ReadAllText((Join-Path $keptDirectory 'notes.txt')) -eq 'not turbo') 'Uninstall removed a file it does not own.'
  Assert-True (Test-UserPathContains -Directory $keptDirectory) 'Uninstall removed the user PATH entry of a directory that still has other files.'
  Assert-True (Test-Path -LiteralPath (Join-Path $directoryDestination 'turbo.exe') -PathType Container) 'Uninstall removed a turbo.exe that is not a regular file.'
  Assert-True ([IO.File]::ReadAllText($profilePath) -ceq $unmarkedProfile) 'Uninstall changed an unmarked profile line.'
  Assert-True ([IO.File]::ReadAllText($editionProfile) -ceq $foreignProfile) "Uninstall changed another installation's hook line."
  Assert-True ((Test-Path -LiteralPath $otherShim) -and -not (Test-Path -LiteralPath $otherMarker)) 'Uninstall removed or executed a turbo it does not own.'
  foreach ($message in @(
    "Left $keptDirectory on the user PATH because it still contains other files.",
    "Left lines in $editionProfile that are marked by the installer",
    "Left $directoryDestination\turbo.exe because it is not a regular file.",
    "Left $otherShim because the installer did not install it."
  )) {
    Assert-True ($output.Contains($message)) "Uninstall did not report '$message'. Received: $output"
  }
  $env:Path = $systemPath
  Write-Output 'PASS: Windows uninstall leaves binaries, files, PATH entries, and profile lines it does not own.'
} finally {
  if (Test-Path -LiteralPath $testRoot) {
    Remove-Item -LiteralPath $testRoot -Recurse -Force
  }
  if ([string]::IsNullOrWhiteSpace($previousGitHubPath)) {
    Remove-Item Env:GITHUB_PATH -ErrorAction SilentlyContinue
  } else {
    $env:GITHUB_PATH = $previousGitHubPath
  }
  if ([string]::IsNullOrWhiteSpace($previousTurboVersion)) {
    Remove-Item Env:TURBO_VERSION -ErrorAction SilentlyContinue
  } else {
    $env:TURBO_VERSION = $previousTurboVersion
  }
  if ([string]::IsNullOrWhiteSpace($previousTurboInstallDirectory)) {
    Remove-Item Env:TURBO_INSTALL_DIR -ErrorAction SilentlyContinue
  } else {
    $env:TURBO_INSTALL_DIR = $previousTurboInstallDirectory
  }
  $env:TURBO_NO_MODIFY_PATH = $previousNoModifyPath
  $env:TURBO_NO_SHELL_HOOK = $previousNoShellHook
  $env:TURBO_UNINSTALL = $previousUninstall
  $env:LOCALAPPDATA = $previousLocalAppData
  if ($null -eq $previousUserPath) {
    $environmentKey.DeleteValue('Path', $false)
  } else {
    $environmentKey.SetValue('Path', $previousUserPath, $previousUserPathKind)
  }
  $environmentKey.Dispose()
  $env:Path = $previousPath
}
