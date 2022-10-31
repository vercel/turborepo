param(
    [bool]$IncludeWin81Sdk = $false
)

$stopWatch = [Diagnostics.Stopwatch]::StartNew()
$installerArgs = @("--norestart","--passive","--wait","--includeRecommended","--add Microsoft.VisualStudio.Workload.VCTools")
$installerBinary = "C:\vs_buildtools_15.exe"

if ($IncludeWin81Sdk -eq $true) {
    $installerArgs += "--add Microsoft.VisualStudio.Component.Windows81SDK"
}

Invoke-WebRequest "https://aka.ms/vs/15/release/vs_BuildTools.exe" -OutFile $installerBinary

$installerProcess = Start-Process -Wait -PassThru -FilePath $installerBinary -ArgumentList $installerArgs
if ($installerProcess.ExitCode -ne 0) {
    exit $installerProcess.ExitCode;
}

$resolvedBuildToolsItems = @(Get-VSSetupInstance | Where-Object { $_.DisplayName -eq "Visual Studio Build Tools 2017" })
if ($resolvedBuildToolsItems.Count -eq 0) {
    Write-Output "Failed to install: Microsoft.VisualStudio.Workload.VCTools."
    exit 1
}
Write-Output $resolvedBuildToolsItems

if ($IncludeWin81Sdk -eq $true) {
    $resolvedWin81SdkItems = @($(Get-VSSetupInstance | Select-VSSetupInstance -Product *).Packages | Where-Object Id -eq "Microsoft.VisualStudio.Component.Windows81SDK")
    if ($resolvedWin81SdkItems.Count -eq 0) {
        Write-Output "Failed to install: Microsoft.VisualStudio.Component.Windows81SDK."
        exit 1
    }
    Write-Output $resolvedWin81SdkItems
}

$stopWatch.Stop()
$stopWatch.Elapsed