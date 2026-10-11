param(
    [Parameter(Mandatory = $true)]
    [string]$OutputPath
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Set-Location $repoRoot

$metadataText = & cargo metadata --locked --filter-platform x86_64-pc-windows-msvc --format-version 1
if ($LASTEXITCODE -ne 0) {
    throw 'cargo metadata failed; third-party notices were not generated.'
}
$metadata = ($metadataText -join "`n") | ConvertFrom-Json
$packages = @($metadata.packages | Where-Object { $_.source -and $_.source.StartsWith('registry+') } | Sort-Object name, version)

$notice = [System.Text.StringBuilder]::new()
[void]$notice.AppendLine('# Third-party notices')
[void]$notice.AppendLine()
[void]$notice.AppendLine('This release statically links the Rust crates listed below. License identifiers are taken from each crate manifest. License/notice files are copied from the exact registry source used by Cargo when present.')
[void]$notice.AppendLine()
[void]$notice.AppendLine('This Cargo notice does not grant redistribution rights for the separately packaged `assets/` artwork and media. Maintainers must verify and record those asset rights before publishing a public release.')
[void]$notice.AppendLine()
[void]$notice.AppendLine('Some registry archives omit their license text files. For those crates, the declared SPDX expression and upstream repository are recorded below; a release reviewer must obtain/verify the corresponding text before making the draft release public.')
[void]$notice.AppendLine()
[void]$notice.AppendLine('Sono-Renderer itself is distributed under the MIT License in `LICENSE`. FFmpeg is not bundled in these release artifacts. On Windows, the application can download the separate Gyan FFmpeg distribution on first MP4 export; see `docs/INSTALLATION.md` for its upstream license and download behavior.')
[void]$notice.AppendLine()

foreach ($package in $packages) {
    $packageRoot = Split-Path -Parent $package.manifest_path
    $licenseFiles = @()
    if ($package.license_file) {
        $declared = [string]$package.license_file
        if ([System.IO.Path]::IsPathRooted($declared)) {
            $candidate = $declared
        } else {
            $candidate = Join-Path $packageRoot $declared
        }
        if (Test-Path -LiteralPath $candidate -PathType Leaf) {
            $licenseFiles += Get-Item -LiteralPath $candidate
        }
    }
    if ($licenseFiles.Count -eq 0) {
        $licenseFiles = @(Get-ChildItem -LiteralPath $packageRoot -File | Where-Object {
            $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE)(\.|$|[-_])'
        } | Sort-Object Name)
    }

    $licenseName = if ($package.license) { [string]$package.license } else { 'not declared in Cargo metadata' }
    [void]$notice.AppendLine("## $($package.name) $($package.version)")
    [void]$notice.AppendLine()
    [void]$notice.AppendLine("Declared license: $licenseName")
    [void]$notice.AppendLine()

    if ($licenseFiles.Count -eq 0) {
        if (-not $package.license) {
            throw "Registry package $($package.name) $($package.version) has neither a license file nor an SPDX expression. Resolve this before packaging."
        }
        [void]$notice.AppendLine('No license text file was included in this registry archive. Obtain and verify the text from the upstream repository before publishing this draft release.')
        if ($package.repository) {
            [void]$notice.AppendLine("Upstream: $($package.repository)")
        }
        [void]$notice.AppendLine()
        continue
    }

    foreach ($licenseFile in $licenseFiles) {
        [void]$notice.AppendLine("### $($licenseFile.Name)")
        [void]$notice.AppendLine()
        $text = [System.IO.File]::ReadAllText($licenseFile.FullName)
        $escaped = $text.Replace('&', '&amp;').Replace('<', '&lt;').Replace('>', '&gt;')
        [void]$notice.AppendLine('<details><summary>License text</summary><pre>')
        [void]$notice.AppendLine($escaped)
        [void]$notice.AppendLine('</pre></details>')
        [void]$notice.AppendLine()
    }
}

$outputFullPath = [System.IO.Path]::GetFullPath($OutputPath)
$parent = Split-Path -Parent $outputFullPath
New-Item -ItemType Directory -Force -Path $parent | Out-Null
[System.IO.File]::WriteAllText($outputFullPath, $notice.ToString(), [System.Text.UTF8Encoding]::new($false))
Write-Output "Wrote third-party notices for $($packages.Count) Cargo registry packages to $outputFullPath"
