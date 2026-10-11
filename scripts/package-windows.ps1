param(
    [ValidateSet('development', 'stable')]
    [string]$Channel = 'development',
    [string]$OutputRoot = 'target',
    [switch]$SkipBuild,
    [switch]$RequireInstaller
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Set-Location $repoRoot
$repoPrefix = $repoRoot.TrimEnd('\') + '\'

$outputRootPath = if ([System.IO.Path]::IsPathRooted($OutputRoot)) {
    [System.IO.Path]::GetFullPath($OutputRoot)
} else {
    [System.IO.Path]::GetFullPath((Join-Path $repoRoot $OutputRoot))
}
if (-not $outputRootPath.StartsWith($repoPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Package output must stay inside the repository: $outputRootPath"
}
$ancestor = $outputRootPath
while ($ancestor.StartsWith($repoPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
    if (Test-Path -LiteralPath $ancestor) {
        $item = Get-Item -LiteralPath $ancestor -Force
        if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "Package output cannot be below a reparse point: $ancestor"
        }
    }
    $ancestor = Split-Path -Parent $ancestor
}

function Invoke-Checked([string]$Executable, [string[]]$Arguments) {
    & $Executable @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Executable exited with code $LASTEXITCODE"
    }
}

$cargoToml = Get-Content -LiteralPath (Join-Path $repoRoot 'Cargo.toml') -Raw
if ($cargoToml -notmatch '(?m)^version\s*=\s*"([^"]+)"') {
    throw 'Could not read the application version from Cargo.toml.'
}
$version = $Matches[1]
if ($version -notmatch '^\d+\.\d+\.\d+$') {
    throw "Release packaging requires a stable numeric Cargo version; found $version."
}

$status = @(git status --porcelain=v1 --untracked-files=normal)
if ($LASTEXITCODE -ne 0) {
    throw 'Could not inspect Git worktree state.'
}
$dirty = $status.Count -gt 0
$commit = if ($env:GITHUB_SHA) { $env:GITHUB_SHA } else { (git rev-parse HEAD).Trim() }
if ($LASTEXITCODE -ne 0 -or -not $commit) {
    throw 'Could not determine the source commit for build identity.'
}
if ($Channel -eq 'stable') {
    if ($dirty) { throw 'Stable artifacts must be built from a clean source checkout.' }
    if ($env:GITHUB_REF_NAME -ne "v$version") {
        throw "Stable tag $($env:GITHUB_REF_NAME) does not match Cargo.toml version v$version."
    }
}

$env:SONO_RENDERER_GIT_COMMIT = $commit
$env:SONO_RENDERER_BUILD_DIRTY = if ($dirty) { 'true' } else { 'false' }
$env:SONO_RENDERER_BUILD_CHANNEL = $Channel
$env:SONO_RENDERER_REQUIRE_WIN_RESOURCE = if ($Channel -eq 'stable') { 'true' } else { 'false' }

$distribution = Join-Path $outputRootPath 'distribution'
$stageRoot = Join-Path $outputRootPath 'release-stage'
foreach ($existing in @($distribution, $stageRoot)) {
    if (Test-Path -LiteralPath $existing) {
        throw "Refusing to replace existing package output '$existing'. Choose a fresh -OutputRoot or archive that output explicitly."
    }
}
$installedStage = Join-Path $stageRoot 'installed'
$portableStage = Join-Path $stageRoot 'portable'
New-Item -ItemType Directory -Force -Path $installedStage, $portableStage, $distribution | Out-Null

if (-not $SkipBuild) {
    Invoke-Checked 'cargo' @('build', '--locked', '--release', '--bin', 'renderer')
}
$binary = Join-Path $repoRoot 'target\release\renderer.exe'
if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
    throw "Release executable is missing: $binary"
}
if ($Channel -eq 'stable') {
    $versionInfo = [System.Diagnostics.FileVersionInfo]::GetVersionInfo($binary)
    if ($versionInfo.ProductName -ne 'Sono-Renderer' -or $versionInfo.ProductVersion -ne $version) {
        throw 'The stable Windows executable does not contain matching Sono-Renderer version metadata.'
    }
}

$versionedPackage = [ordered]@{
    schema_version = 1
    product = 'Sono-Renderer'
    version = $version
    channel = $Channel
    commit = $commit.Substring(0, [Math]::Min(12, $commit.Length))
    dirty = $dirty
    target = 'x86_64-pc-windows-msvc'
    build_profile = 'release'
    rustc = (& rustc --version).Trim()
    portable_data_marker = 'portable.mode'
}
$buildInfo = $versionedPackage | ConvertTo-Json -Depth 4

foreach ($destination in @($installedStage, $portableStage)) {
    Copy-Item -LiteralPath $binary -Destination (Join-Path $destination 'Sono-Renderer.exe')
    Copy-Item -LiteralPath (Join-Path $repoRoot 'assets') -Destination $destination -Recurse
    Copy-Item -LiteralPath (Join-Path $repoRoot 'ui-profiles') -Destination $destination -Recurse
    Copy-Item -LiteralPath (Join-Path $repoRoot 'watermark-profiles') -Destination $destination -Recurse
    Copy-Item -LiteralPath (Join-Path $repoRoot 'locales') -Destination $destination -Recurse
    Copy-Item -LiteralPath (Join-Path $repoRoot 'LICENSE') -Destination $destination
    Copy-Item -LiteralPath (Join-Path $repoRoot 'docs\GETTING_STARTED.md') -Destination $destination
    Copy-Item -LiteralPath (Join-Path $repoRoot 'locales\README.md') -Destination (Join-Path $destination 'LOCALIZATION.md')
    Copy-Item -LiteralPath (Join-Path $repoRoot 'docs\WORKSPACE_COMMANDS.md') -Destination $destination
    Copy-Item -LiteralPath (Join-Path $repoRoot 'docs\TROUBLESHOOTING.md') -Destination $destination
    Copy-Item -LiteralPath (Join-Path $repoRoot 'docs\INSTALLATION.md') -Destination $destination
    Copy-Item -LiteralPath (Join-Path $repoRoot 'docs\CHART_STUDIO.md') -Destination $destination
    Copy-Item -LiteralPath (Join-Path $repoRoot 'docs\PRESENTATION_STUDIO.md') -Destination $destination
    Copy-Item -LiteralPath (Join-Path $repoRoot 'docs\PRESENTATION_STUDIO_SUBTITLES.md') -Destination $destination
    Copy-Item -LiteralPath (Join-Path $repoRoot 'docs\PRESENTATION_STUDIO_ASSETS.md') -Destination $destination
    Copy-Item -LiteralPath (Join-Path $repoRoot 'docs\MEDIA_ACQUISITION.md') -Destination $destination
    Copy-Item -LiteralPath (Join-Path $repoRoot 'docs\KNOWN_LIMITATIONS.md') -Destination $destination
    [System.IO.File]::WriteAllText((Join-Path $destination 'build-info.json'), $buildInfo, [System.Text.UTF8Encoding]::new($false))
}
& (Join-Path $PSScriptRoot 'generate-third-party-notices.ps1') -OutputPath (Join-Path $installedStage 'THIRD_PARTY_NOTICES.md')
if ($LASTEXITCODE -ne 0) { throw 'Third-party notice generation failed.' }
Copy-Item -LiteralPath (Join-Path $installedStage 'THIRD_PARTY_NOTICES.md') -Destination $portableStage
[System.IO.File]::WriteAllText((Join-Path $portableStage 'portable.mode'), "sono-renderer-portable-v1`r`n", [System.Text.Encoding]::ASCII)

$portableName = "Sono-Renderer-$version-windows-x86_64-portable.zip"
$portablePath = Join-Path $distribution $portableName
Compress-Archive -Path (Join-Path $portableStage '*') -DestinationPath $portablePath -CompressionLevel Optimal

$compilerPath = (Get-Command 'ISCC.exe' -ErrorAction SilentlyContinue).Source
if (-not $compilerPath) {
    foreach ($candidate in @(
        'C:\Program Files (x86)\Inno Setup 6\ISCC.exe',
        'C:\Program Files\Inno Setup 6\ISCC.exe'
    )) {
        if (Test-Path -LiteralPath $candidate -PathType Leaf) {
            $compilerPath = $candidate
            break
        }
    }
}
if ($compilerPath) {
    Invoke-Checked $compilerPath @(
        "/DAppVersion=$version",
        "/DSourceDir=$installedStage",
        "/DOutputDir=$distribution",
        (Join-Path $repoRoot 'packaging\windows\Sono-Renderer.iss')
    )
} elseif ($RequireInstaller) {
    throw 'Inno Setup 6 compiler ISCC.exe is required but unavailable.'
} else {
    Write-Warning 'Inno Setup 6 was not found; only the portable ZIP was created.'
}

$files = @(Get-ChildItem -LiteralPath $distribution -File | Where-Object {
    $_.Name -match '\.(zip|exe)$'
} | Sort-Object Name)
if ($files.Count -eq 0) { throw 'No release artifacts were created.' }
$artifactRecords = @($files | ForEach-Object {
    [ordered]@{
        file = $_.Name
        bytes = $_.Length
        sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    }
})
$manifest = [ordered]@{
    schema_version = 1
    product = 'Sono-Renderer'
    version = $version
    channel = $Channel
    commit = $commit
    dirty = $dirty
    target = 'x86_64-pc-windows-msvc'
    platform_status = 'Windows x86_64 packaged; non-GUI artifact smoke tests run in CI'
    reproducible_build = $false
    artifacts = $artifactRecords
}
$manifestPath = Join-Path $distribution "Sono-Renderer-$version-release-manifest.json"
[System.IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 8), [System.Text.UTF8Encoding]::new($false))
$sumLines = @($artifactRecords | ForEach-Object { "$($_.sha256)  $($_.file)" })
[System.IO.File]::WriteAllLines((Join-Path $distribution 'SHA256SUMS.txt'), $sumLines, [System.Text.UTF8Encoding]::new($false))

Write-Output "Created Windows artifacts in $distribution"
Get-ChildItem -LiteralPath $distribution -File | Select-Object Name, Length
