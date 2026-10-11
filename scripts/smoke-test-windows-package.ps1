param(
    [Parameter(Mandatory = $true)]
    [string]$DistributionDirectory,
    [switch]$PortableOnly
)

$ErrorActionPreference = 'Stop'
$distribution = (Resolve-Path -LiteralPath $DistributionDirectory).Path
$runnerTempCandidate = $env:RUNNER_TEMP
if ([string]::IsNullOrWhiteSpace($runnerTempCandidate)) {
    $runnerTempCandidate = [System.IO.Path]::GetTempPath()
}
New-Item -ItemType Directory -Force -Path $runnerTempCandidate | Out-Null
$runnerTemp = (Resolve-Path -LiteralPath $runnerTempCandidate).Path
$testRoot = Join-Path $runnerTemp ("Sono Renderer release smoke #28 " + [Guid]::NewGuid().ToString('N'))
$unicodeName = 'M' + [char]0x00fa + 'sica'
$portableRoot = Join-Path $testRoot ("Portable $unicodeName path with spaces # smoke")
$projectRoot = Join-Path $testRoot 'Creator Projects - preserve'
$localAppData = $env:LOCALAPPDATA
$originalPath = $env:PATH
$passed = $false

function Invoke-App([string]$Executable, [string[]]$Arguments) {
    $previousErrorAction = $ErrorActionPreference
    try {
        # Windows PowerShell promotes native stderr to ErrorRecord values. The
        # renderer writes useful CLI context there, so capture it and rely on
        # the actual process exit code for success/failure.
        $ErrorActionPreference = 'Continue'
        $output = & $Executable @Arguments 2>&1 | Out-String -Width 4096
        $exitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousErrorAction
    }
    if ($exitCode -ne 0) {
        throw "$Executable $($Arguments -join ' ') failed with exit code $exitCode`n$output"
    }
    return $output
}

function Invoke-Installer([string]$Installer, [string]$InstallDirectory) {
    $args = "/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /CURRENTUSER /DIR=`"$InstallDirectory`""
    $process = Start-Process -FilePath $Installer -ArgumentList $args -Wait -PassThru -NoNewWindow
    if ($process.ExitCode -ne 0) {
        throw "Installer exited with code $($process.ExitCode)."
    }
}

try {
    New-Item -ItemType Directory -Force -Path $testRoot, $portableRoot, $projectRoot | Out-Null
    $zip = Get-ChildItem -LiteralPath $distribution -Filter '*-portable.zip' | Select-Object -First 1
    if (-not $zip) { throw 'Portable ZIP was not found.' }
    Expand-Archive -LiteralPath $zip.FullName -DestinationPath $portableRoot
    $portableExe = Join-Path $portableRoot 'Sono-Renderer.exe'
    if (-not (Test-Path -LiteralPath $portableExe)) { throw 'Portable executable is missing.' }
    if ((Get-Content -LiteralPath (Join-Path $portableRoot 'portable.mode') -Raw).Trim() -ne 'sono-renderer-portable-v1') {
        throw 'Portable mode marker is invalid.'
    }
    if (-not (Test-Path -LiteralPath (Join-Path $portableRoot 'assets\presentation-pack.json'))) {
        throw 'Packaged presentation assets are missing.'
    }
    if (-not (Test-Path -LiteralPath (Join-Path $portableRoot 'MEDIA_ACQUISITION.md'))) {
        throw 'Packaged media acquisition documentation is missing.'
    }
    & (Join-Path $PSScriptRoot 'validate-locales.ps1') -LocaleRoot (Join-Path $portableRoot 'locales')
    if (-not (Test-Path -LiteralPath (Join-Path $portableRoot 'LOCALIZATION.md'))) {
        throw 'Packaged localization guidance is missing.'
    }

    $env:LOCALAPPDATA = Join-Path $testRoot 'local app data should not be used by portable mode'
    $env:PATH = $env:WINDIR + '\System32'
    Push-Location (Join-Path $env:WINDIR 'System32')
    try {
        $versionOutput = Invoke-App $portableExe @('--version')
        if ($versionOutput -notmatch 'Sono-Renderer\s+\d+\.\d+\.\d+') {
            throw "Unexpected packaged CLI version output: $versionOutput"
        }
        [void](Invoke-App $portableExe @('--help'))
        $queueOutput = Invoke-App $portableExe @('jobs', 'list')
        $mediaToolsOutput = Invoke-App $portableExe @('media', 'tool-info', '--json')
        $mediaTools = $mediaToolsOutput | ConvertFrom-Json
        if ($mediaTools.pinned_version -ne '2026.08.19') {
            throw "Unexpected packaged yt-dlp pin: $($mediaTools.pinned_version)"
        }
        if ($mediaTools.available) {
            throw 'A clean portable test profile unexpectedly had a managed yt-dlp binary.'
        }
        $expectedPortableState = Join-Path $portableRoot 'UserData\jobs.json'
        if ($queueOutput -notlike "*${expectedPortableState}*") {
            throw "Portable job state did not resolve beside the archive: $queueOutput"
        }
    } finally {
        Pop-Location
    }

    if (-not $PortableOnly) {
        $installer = Get-ChildItem -LiteralPath $distribution -Filter '*-setup.exe' | Select-Object -First 1
        if (-not $installer) { throw 'Windows installer was not found.' }
        $installDirectory = Join-Path $testRoot 'Installed path with spaces # smoke'
        Invoke-Installer $installer.FullName $installDirectory
        $installedExe = Join-Path $installDirectory 'Sono-Renderer.exe'
        if (-not (Test-Path -LiteralPath $installedExe)) { throw 'Installer did not place the executable.' }
        & (Join-Path $PSScriptRoot 'validate-locales.ps1') -LocaleRoot (Join-Path $installDirectory 'locales')
        if (-not (Test-Path -LiteralPath (Join-Path $installDirectory 'LOCALIZATION.md'))) {
            throw 'Installed localization guidance is missing.'
        }

        $env:LOCALAPPDATA = $localAppData
        $env:PATH = $env:WINDIR + '\System32'
        Push-Location (Join-Path $env:WINDIR 'System32')
        try {
            $installedQueue = Invoke-App $installedExe @('jobs', 'list')
            $expectedInstalledState = Join-Path $localAppData 'Sono-Renderer\jobs.json'
            if ($installedQueue -notlike "*${expectedInstalledState}*") {
                throw "Installed data did not resolve to per-user application data: $installedQueue"
            }
        } finally {
            Pop-Location
        }

        $preservationMarker = Join-Path $localAppData 'Sono-Renderer\release-smoke-user-data.txt'
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $preservationMarker) | Out-Null
        [System.IO.File]::WriteAllText($preservationMarker, 'keep')
        $project = Join-Path $projectRoot 'chart ñ - keep.sonoproject'
        [System.IO.File]::WriteAllText($project, 'project data remains external to the install')

        # Reinstalling the same version exercises the installer's repair/reinstall path.
        Invoke-Installer $installer.FullName $installDirectory
        if (-not (Test-Path -LiteralPath $preservationMarker)) { throw 'Reinstall removed per-user data.' }

        $uninstaller = Join-Path $installDirectory 'unins000.exe'
        if (-not (Test-Path -LiteralPath $uninstaller)) { throw 'Uninstaller registration executable is missing.' }
        $uninstall = Start-Process -FilePath $uninstaller -ArgumentList '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART' -Wait -PassThru -NoNewWindow
        if ($uninstall.ExitCode -ne 0) { throw "Uninstaller exited with code $($uninstall.ExitCode)." }
        if (Test-Path -LiteralPath $installedExe) { throw 'Uninstall left the application executable behind.' }
        if (-not (Test-Path -LiteralPath $preservationMarker)) { throw 'Uninstall removed application user data.' }
        if ((Get-Content -LiteralPath $project -Raw) -ne 'project data remains external to the install') {
            throw 'Uninstall modified an external user project.'
        }
        $appPathKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\App Paths\Sono-Renderer.exe'
        if (Test-Path -LiteralPath $appPathKey) { throw 'Uninstall left the per-user App Paths registration behind.' }
    }
    $passed = $true
    if ($PortableOnly) {
        Write-Output 'PASS: portable extraction, Unicode/space package path, foreign working directory, version/help/jobs/media-tool-info CLI.'
    } else {
        Write-Output 'PASS: portable extraction, foreign working directory, Unicode/space paths, version/help/jobs/media-tool-info CLI, installer, same-version reinstall, and uninstall preserve user data/projects.'
    }
} finally {
    $env:LOCALAPPDATA = $localAppData
    $env:PATH = $originalPath
    if ($passed) {
        $fullRoot = [System.IO.Path]::GetFullPath($testRoot)
        $tempPrefix = $runnerTemp.TrimEnd('\') + '\'
        if (-not $fullRoot.StartsWith($tempPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
            throw "Refusing to remove smoke-test directory outside RUNNER_TEMP: $fullRoot"
        }
        Remove-Item -LiteralPath $fullRoot -Recurse -Force
    } else {
        Write-Warning "Smoke-test files retained for inspection at $testRoot"
    }
}
