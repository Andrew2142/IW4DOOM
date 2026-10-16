# SPDX-License-Identifier: GPL-3.0-or-later
# Failure-mode tests for iw4doom.bat / iw4doom.ps1 (sheet 15 T15.3). Windows only; runs in CI.
# Needs no game data: the fake MW2 install is two empty files, the IWAD is Freedoom
# (tools/fetch-freedoom.ps1). Every case must stop before building anything.
# Goes through iw4doom.bat, so the script runs under Windows PowerShell 5.1 as for players.
# Keep this file ASCII.

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$Root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$Bat = Join-Path $Root 'iw4doom.bat'
$Freedoom = Join-Path $Root 'wads\freedoom1.wad'
if (-not (Test-Path -LiteralPath $Freedoom)) {
    Write-Host "test-launcher: $Freedoom missing; run tools\fetch-freedoom.ps1"
    exit 1
}

$Tmp = Join-Path ([IO.Path]::GetTempPath()) ('iw4doom-launcher-' + [Guid]::NewGuid().ToString('N'))
$Empty = Join-Path $Tmp 'empty'
$Fd = Join-Path $Tmp 'fd'
$Fake = Join-Path $Tmp 'fake'
$Home2 = Join-Path $Tmp 'home'
foreach ($d in @($Empty, (Join-Path $Fd 'doom'), (Join-Path $Fake 'doom'), (Join-Path $Fake 'iw4\zone\english'), $Home2)) {
    New-Item -ItemType Directory -Force -Path $d | Out-Null
}
Copy-Item -LiteralPath $Freedoom -Destination (Join-Path $Fd 'doom')
Copy-Item -LiteralPath $Freedoom -Destination (Join-Path $Fake 'doom')
New-Item -ItemType File -Force -Path (Join-Path $Fake 'iw4\zone\english\common_mp.ff') | Out-Null
New-Item -ItemType File -Force -Path (Join-Path $Fake 'iw4\zone\english\mp_rust.ff') | Out-Null

$script:Failures = 0
# Never stop at an install prompt, also when run by hand outside CI.
$env:IW4DOOM_NONINTERACTIVE = '1'

# Runs iw4doom.bat with $BatArgs (and $EnvVars applied for this call) and checks exit code and output.
function Expect([string]$Name, [int]$WantExit, [string[]]$Needles, [string[]]$Unwanted,
    [string[]]$BatArgs, [hashtable]$EnvVars = @{}) {
    $saved = @{}
    foreach ($k in $EnvVars.Keys) {
        $saved[$k] = [Environment]::GetEnvironmentVariable($k)
        [Environment]::SetEnvironmentVariable($k, $EnvVars[$k])
    }
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $out = (& $Bat @BatArgs 2>&1 | ForEach-Object { [string]$_ }) -join "`n"
        $code = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previous
        foreach ($k in $saved.Keys) { [Environment]::SetEnvironmentVariable($k, $saved[$k]) }
    }
    $problems = New-Object System.Collections.Generic.List[string]
    if ($code -ne $WantExit) { $problems.Add("exit status $code, expected $WantExit") }
    foreach ($n in $Needles) { if (-not $out.Contains($n)) { $problems.Add("missing from output: $n") } }
    foreach ($n in $Unwanted) { if ($out.Contains($n)) { $problems.Add("unexpected in output: $n") } }
    if ($problems.Count -eq 0) {
        Write-Host "ok    $Name"
    } else {
        Write-Host "FAIL  $Name"
        foreach ($p in $problems) { Write-Host "  $p" }
        Write-Host '  output was:'
        foreach ($line in ($out -split "`n")) { Write-Host "      | $line" }
        $script:Failures += 1
    }
}

try {
    Expect '--help prints usage' 0 @('Usage: iw4doom.bat', '--setup-only') @() @('--help')
    Expect '-Help (PowerShell spelling) prints usage' 0 @('Usage: iw4doom.bat') @() @('-Help')
    Expect 'empty games dir names both missing items' 1 `
        @('[1/8]', "missing: $Empty\iw4\", 'missing: a Doom IWAD', 'mklink /J', 'games\README.md') `
        @('[2/8]') @('--games', $Empty)
    Expect 'Freedoom only names games\iw4 and the junction command' 1 `
        @("missing: $Fd\iw4\", 'mklink /J') @('a Doom IWAD', '[2/8]') @('--games', $Fd)
    Expect '--list shows the IWAD' 0 @('freedoom1.wad  (default)') @() @('--games', $Fd, '--list')
    Expect '-List (PowerShell spelling) shows the IWAD' 0 @('freedoom1.wad  (default)') @() @('-Games', $Fd, '-List')
    Expect '--iwad with an unknown name' 1 @("no IWAD 'nope'") @() @('--games', $Fake, '--iwad', 'nope')

    # No Rust: hide cargo/rustup from PATH and from the ~/.cargo\bin fallback.
    $cleanPath = (($env:Path -split ';') | Where-Object {
            $_ -and -not (Test-Path -LiteralPath (Join-Path $_ 'cargo.exe')) -and
            -not (Test-Path -LiteralPath (Join-Path $_ 'rustup.exe'))
        }) -join ';'
    Expect 'no Rust prints the rustup-init instructions' 1 `
        @('[2/8]', 'Rust is not installed', 'https://rustup.rs') @('[3/8]', 'Install Rust now?') `
        @('--games', $Fake) `
        @{ Path = $cleanPath; CARGO_HOME = (Join-Path $Home2 '.cargo'); RUSTUP_HOME = (Join-Path $Home2 '.rustup'); USERPROFILE = $Home2 }
} finally {
    Remove-Item -LiteralPath $Tmp -Recurse -Force -ErrorAction SilentlyContinue
}

if ($script:Failures -gt 0) {
    Write-Host "test-launcher: $($script:Failures) case(s) failed"
    exit 1
}
Write-Host 'test-launcher: all cases passed'
