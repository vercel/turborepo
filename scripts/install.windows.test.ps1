$ErrorActionPreference = 'Stop'
$installerPath = Join-Path $PSScriptRoot '..\apps\docs\public\install.ps1'
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ("turbo-installer-test-" + [Guid]::NewGuid().ToString('N'))
$previousGitHubPath = $env:GITHUB_PATH
$previousTurboVersion = $env:TURBO_VERSION
$previousTurboInstallDirectory = $env:TURBO_INSTALL_DIR
$previousPath = $env:Path
$script:RequestCount = 0
$script:ChecksumFixture = $null
$script:ArchiveFixture = $null
$script:ArchiveName = $null

function Assert-True {
  param([bool]$Condition, [string]$Message)
  if (-not $Condition) {
    throw $Message
  }
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
  $env:Path = $previousPath
}
