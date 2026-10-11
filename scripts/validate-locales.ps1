param(
    [string]$LocaleRoot = (Join-Path $PSScriptRoot '..\locales')
)

$ErrorActionPreference = 'Stop'
$LocaleRoot = (Resolve-Path -LiteralPath $LocaleRoot).Path
$Utf8 = [System.Text.UTF8Encoding]::new($false, $true)
$MaxCatalogBytes = 2MB
$MaxRegistryBytes = 64KB

function Read-JsonFile([string]$Path, [long]$Limit) {
    $info = Get-Item -LiteralPath $Path
    if ($info.Length -gt $Limit) { throw "JSON resource exceeds size limit: $Path" }
    $text = [System.IO.File]::ReadAllText($Path, $Utf8)
    try { return ,($text | ConvertFrom-Json) }
    catch { throw "Invalid UTF-8 JSON in $Path`: $($_.Exception.Message)" }
}

function Convert-JsonObjectToDictionary($Object) {
    if ($null -eq $Object -or $null -eq $Object.PSObject) { throw 'Expected a JSON object.' }
    $dictionary = [System.Collections.Specialized.OrderedDictionary]::new([System.StringComparer]::Ordinal)
    foreach ($property in $Object.PSObject.Properties) {
        if ($dictionary.Contains($property.Name)) { throw "Duplicate JSON key '$($property.Name)'." }
        $dictionary.Add($property.Name, $property.Value)
    }
    return ,$dictionary
}

function Get-Placeholders([string]$Message, [string]$Context) {
    $matches = [regex]::Matches($Message, '\{([A-Za-z0-9_.]+)\}')
    $remainder = [regex]::Replace($Message, '\{[A-Za-z0-9_.]+\}', '')
    if ($remainder.Contains('{') -or $remainder.Contains('}')) {
        throw "Malformed placeholder in $Context"
    }
    return @($matches | ForEach-Object { $_.Groups[1].Value } | Sort-Object -Unique)
}

$registryPath = Join-Path $LocaleRoot 'registry.json'
$registry = Read-JsonFile $registryPath $MaxRegistryBytes
if ($registry.schema_version -ne 1) { throw "Unsupported locale registry schema $($registry.schema_version)." }
if (-not $registry.locales -or $registry.locales.Count -lt 1) { throw 'Locale registry is empty.' }

$byId = @{}
foreach ($locale in $registry.locales) {
    if ([string]::IsNullOrWhiteSpace($locale.id) -or $byId.ContainsKey($locale.id)) {
        throw "Missing or duplicate locale ID: '$($locale.id)'"
    }
    if ($locale.id -notmatch '^[A-Za-z0-9-]{1,35}$') { throw "Invalid locale ID: $($locale.id)" }
    if ($locale.direction -notin @('ltr', 'rtl')) { throw "Invalid direction for $($locale.id)" }
    if ($locale.audience -notin @('release', 'developer')) { throw "Invalid audience for $($locale.id)" }
    if ([string]::IsNullOrWhiteSpace($locale.native_name) -or [string]::IsNullOrWhiteSpace($locale.fallback)) {
        throw "Locale $($locale.id) needs a native name and fallback."
    }
    $byId[$locale.id] = $locale
}

$releaseIds = @($registry.locales | Where-Object audience -eq 'release' | ForEach-Object id)
if ($releaseIds.Count -ne 10) { throw "Expected 10 release locales; found $($releaseIds.Count)." }
foreach ($required in @('en', 'ja', 'zh-CN', 'zh-TW', 'ko', 'es', 'pt-BR', 'fr', 'de', 'ru')) {
    if (-not $byId.ContainsKey($required) -or $byId[$required].audience -ne 'release') {
        throw "Required release locale is missing: $required"
    }
}

$catalogs = @{}
foreach ($locale in $registry.locales) {
    if ($locale.id -eq 'qps-ploc') { $path = Join-Path $LocaleRoot 'test\pseudo\messages.json' }
    elseif ($locale.id -eq 'qps-rtl') { $path = Join-Path $LocaleRoot 'test\rtl\messages.json' }
    else { $path = Join-Path (Join-Path $LocaleRoot $locale.id) 'messages.json' }
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Catalog missing: $path" }
    $catalog = Convert-JsonObjectToDictionary (Read-JsonFile $path $MaxCatalogBytes)
    $catalogs[$locale.id] = $catalog
    $cursor = $locale
    $visited = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)
    while ($cursor.fallback -ne $cursor.id) {
        if (-not $visited.Add($cursor.id)) { throw "Fallback cycle includes $($cursor.id)." }
        if (-not $byId.ContainsKey($cursor.fallback)) { throw "Unknown fallback '$($cursor.fallback)' for $($cursor.id)." }
        $cursor = $byId[$cursor.fallback]
    }
}

$english = $catalogs['en']
if (-not $english -or -not $english.Contains('app.name')) { throw 'Canonical English catalog is missing app.name.' }
foreach ($locale in $registry.locales | Where-Object audience -eq 'release') {
    $catalog = $catalogs[$locale.id]
    foreach ($key in $english.Keys) {
        if (-not $catalog.Contains($key)) { throw "Locale $($locale.id) is missing canonical key '$key'." }
        $expected = @(Get-Placeholders ([string]$english[$key]) "en:$key")
        $actual = @(Get-Placeholders ([string]$catalog[$key]) "$($locale.id):$key")
        if (($expected -join "`n") -cne ($actual -join "`n")) {
            throw "Placeholder mismatch in $($locale.id):$key ($($expected -join ',') vs $($actual -join ','))."
        }
    }
    foreach ($key in $catalog.Keys) {
        if (-not $english.Contains($key)) { throw "Unknown key '$key' in locale $($locale.id)." }
    }
}

if ($catalogs['zh-CN']['home.project.title'] -ceq $catalogs['zh-TW']['home.project.title']) {
    throw 'Simplified and Traditional Chinese catalogs must be maintained independently.'
}
Write-Output "Locale resources validated: $($releaseIds.Count) release locales, $($registry.locales.Count - $releaseIds.Count) developer locales, $($english.Count) canonical messages."
