# SPDX-License-Identifier: GPL-3.0-or-later
# Windows twin of tools/fetch-freedoom.sh: downloads the pinned Freedoom release into wads\ for
# development and tests. Idempotent. Keep the tag and hashes identical to the .sh.
# Windows PowerShell 5.1 compatible; keep this file ASCII.

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$FreedoomTag = 'v0.13.0'
$FreedoomVersion = $FreedoomTag.TrimStart('v')
$FreedoomZipSha256 = '3f9b264f3e3ce503b4fb7f6bdcb1f419d93c7b546f4df3e874dd878db9688f59'
$Freedoom1Sha256 = '7323bcc168c5a45ff10749b339960e98314740a734c30d4b9f3337001f9e703d'
$Freedoom2Sha256 = 'a8772e088847032510d97ba2312406a6998f21cbab44d4ff10696faa9c0ecd4b'
$Url = "https://github.com/freedoom/freedoom/releases/download/$FreedoomTag/freedoom-$FreedoomVersion.zip"

$Root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$Wads = Join-Path $Root 'wads'

function Get-Sha256([string]$Path) {
    return (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant()
}
function Test-Wad([string]$Name, [string]$Sha) {
    $p = Join-Path $Wads $Name
    return ((Test-Path -LiteralPath $p -PathType Leaf) -and (Get-Sha256 $p) -eq $Sha)
}

if ((Test-Wad 'freedoom1.wad' $Freedoom1Sha256) -and (Test-Wad 'freedoom2.wad' $Freedoom2Sha256)) {
    Write-Host "fetch-freedoom: wads\freedoom1.wad and freedoom2.wad already present (Freedoom $FreedoomVersion)"
    exit 0
}

# Download to a temp dir and move into place only after verification, so a failure never
# leaves a partial WAD in wads\.
$Tmp = Join-Path ([IO.Path]::GetTempPath()) ('fetch-freedoom-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $Tmp | Out-Null
try {
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    $zip = Join-Path $Tmp 'freedoom.zip'
    Write-Host "fetch-freedoom: downloading $Url"
    $ProgressPreference = 'SilentlyContinue'
    Invoke-WebRequest -UseBasicParsing -Uri $Url -OutFile $zip

    $actualZip = Get-Sha256 $zip
    if ($actualZip -ne $FreedoomZipSha256) {
        Write-Host "fetch-freedoom: zip SHA-256 mismatch: expected $FreedoomZipSha256 got $actualZip"
        exit 1
    }

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [IO.Compression.ZipFile]::OpenRead($zip)
    try {
        foreach ($name in @('freedoom1.wad', 'freedoom2.wad')) {
            $entry = $archive.Entries | Where-Object { $_.Name -eq $name } | Select-Object -First 1
            if (-not $entry) {
                Write-Host "fetch-freedoom: $name not found in the zip"
                exit 1
            }
            [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, (Join-Path $Tmp $name), $true)
        }
    } finally {
        $archive.Dispose()
    }

    if ((Get-Sha256 (Join-Path $Tmp 'freedoom1.wad')) -ne $Freedoom1Sha256) {
        Write-Host 'fetch-freedoom: freedoom1.wad SHA-256 mismatch'
        exit 1
    }
    if ((Get-Sha256 (Join-Path $Tmp 'freedoom2.wad')) -ne $Freedoom2Sha256) {
        Write-Host 'fetch-freedoom: freedoom2.wad SHA-256 mismatch'
        exit 1
    }

    New-Item -ItemType Directory -Force -Path $Wads | Out-Null
    Move-Item -Force -LiteralPath (Join-Path $Tmp 'freedoom1.wad') -Destination (Join-Path $Wads 'freedoom1.wad')
    Move-Item -Force -LiteralPath (Join-Path $Tmp 'freedoom2.wad') -Destination (Join-Path $Wads 'freedoom2.wad')
    Write-Host "fetch-freedoom: installed Freedoom $FreedoomVersion into $Wads"
} finally {
    Remove-Item -LiteralPath $Tmp -Recurse -Force -ErrorAction SilentlyContinue
}
