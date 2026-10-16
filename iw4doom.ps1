# SPDX-License-Identifier: GPL-3.0-or-later
# IW4DOOM for Windows: the logic behind iw4doom.bat (docs/specs/15-windows-support.md).
# Run iw4doom.bat (double-click, or from a terminal with options); it starts this script with
#   powershell -NoProfile -ExecutionPolicy Bypass -File iw4doom.ps1 [options] [game arguments]
#
# The first run checks your game files and this machine, offers to install what is missing,
# builds the exporter, turns your Modern Warfare 2 (2009) multiplayer install into a pack under
# packs\mw2, builds the game, runs a headless smoke test and starts the game. Later runs only
# check that everything is up to date and start the game within seconds.
#
# Installs (winget) happen only after you answer yes at a prompt; without a console to ask on
# (or with CI set), it prints what to do and stops. Full output: iw4doom.log.
#
# Windows PowerShell 5.1 compatible. Keep this file ASCII: 5.1 reads BOM-less files as ANSI.

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$ScriptArgs = @($args)
$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
$CallerPwd = (Get-Location).Path
Set-Location -LiteralPath $Root

$Log = Join-Path $Root 'iw4doom.log'
$Steps = 8
$MinRust = [version]'1.97.1'
$ExportZone = 'mp_rust'
# IWAD preference when several are present (sheet 14 section 5.1).
$IwadOrder = @('doom2.wad', 'doom.wad', 'plutonia.wad', 'tnt.wad', 'freedoom2.wad', 'freedoom1.wad', 'doom1.wad')
$Mw2SteamFolder = 'Call of Duty Modern Warfare 2'
$RustupUrl = 'https://rustup.rs'
$BuildToolsUrl = 'https://visualstudio.microsoft.com/visual-cpp-build-tools/'

function Show-Usage {
    Write-Host @'
Usage: iw4doom.bat [options] [game arguments]

Checks and builds what is needed (the first run takes 15-20 minutes), then starts the game.

  --list          List the Doom IWADs in games\doom\ and exit.
  --iwad NAME     Play this IWAD (a name from --list, or a path) instead of the preferred one.
  --games DIR     Folder with your game files (default: games\ in this repository).
                  It must contain iw4\ (MW2 multiplayer install) and doom\ (Doom IWADs);
                  see games\README.md.
  --reexport      Export the MW2 pack again even if packs\mw2 is up to date.
  --clean         Remove previous build output first (cargo clean), then do everything,
                  including the export.
  --setup-only    Do everything except starting the game.
  --help          Show this help.

The PowerShell spellings -List, -Iwad, -Games, -Reexport, -Clean, -SetupOnly work too.
Anything else goes to the game, for example: iw4doom.bat -warp 1 1 -skill 4
'@
}

# ---------------------------------------------------------------------------------------------
# Arguments
# ---------------------------------------------------------------------------------------------
$GamesDir = Join-Path $Root 'games'
$IwadArg = ''
$List = $false
$Reexport = $false
$Clean = $false
$SetupOnly = $false
$GameArgs = New-Object System.Collections.Generic.List[string]
$i = 0
while ($i -lt $ScriptArgs.Count) {
    $a = [string]$ScriptArgs[$i]
    $key = $a.TrimStart('-').ToLowerInvariant()
    $isOption = $a.StartsWith('-')
    if ($a -eq '--') {
        for ($j = $i + 1; $j -lt $ScriptArgs.Count; $j++) { $GameArgs.Add([string]$ScriptArgs[$j]) }
        break
    } elseif ($isOption -and $key -eq 'games') {
        if ($i + 1 -ge $ScriptArgs.Count) { Write-Host 'iw4doom: --games needs a directory'; exit 1 }
        $GamesDir = [string]$ScriptArgs[$i + 1]; $i += 2; continue
    } elseif ($isOption -and $key -eq 'iwad') {
        if ($i + 1 -ge $ScriptArgs.Count) { Write-Host 'iw4doom: --iwad needs a name or path'; exit 1 }
        $IwadArg = [string]$ScriptArgs[$i + 1]; $i += 2; continue
    } elseif ($isOption -and $key -eq 'list') {
        $List = $true
    } elseif ($isOption -and $key -eq 'reexport') {
        $Reexport = $true
    } elseif ($isOption -and $key -eq 'clean') {
        $Clean = $true; $Reexport = $true
    } elseif ($isOption -and ($key -eq 'setup-only' -or $key -eq 'setuponly')) {
        $SetupOnly = $true
    } elseif (($isOption -and ($key -eq 'help' -or $key -eq 'h')) -or $a -eq '/?') {
        Show-Usage; exit 0
    } else {
        $GameArgs.Add($a)
    }
    $i += 1
}
if (-not [IO.Path]::IsPathRooted($GamesDir)) { $GamesDir = Join-Path $CallerPwd $GamesDir }
$GamesDir = [IO.Path]::GetFullPath($GamesDir).TrimEnd('\', '/')

# ---------------------------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------------------------
# A path relative to the repository when it is inside it.
function Show-Path([string]$Path) {
    $prefix = $Root.TrimEnd('\') + '\'
    if ($Path.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
        return $Path.Substring($prefix.Length)
    }
    return $Path
}

function Write-LogRaw([string]$Text) {
    [IO.File]::AppendAllText($Log, $Text + [Environment]::NewLine)
}
function Write-Log([string]$Text) {
    Write-LogRaw ((Get-Date -Format 'yyyy-MM-dd HH:mm:ss') + ' ' + $Text)
}
function Step([int]$N, [string]$Text) {
    Write-Host ("[{0}/{1}] {2}" -f $N, $Steps, $Text)
    Write-Log ("[{0}/{1}] {2}" -f $N, $Steps, $Text)
}
function Info([string]$Text) {
    Write-Host ('      ' + $Text)
    Write-Log ('      ' + $Text)
}
function Fail([string[]]$Lines) {
    foreach ($line in $Lines) {
        Write-Host ('      ' + $line)
        Write-Log ('      ' + $line)
    }
    Write-Host ('      Details: ' + (Show-Path $Log))
    exit 1
}

# Output of the last Invoke-Logged call, one string per line.
$script:LastOutput = @()
# Runs a native command, appending its output to the log. Returns the exit code.
function Invoke-Logged([string]$Exe, [string[]]$Arguments) {
    Write-Log ('$ ' + $Exe + ' ' + ($Arguments -join ' '))
    $lines = New-Object System.Collections.Generic.List[string]
    if (-not (Get-Command $Exe -ErrorAction SilentlyContinue)) {
        Write-Log "not found: $Exe"
        $script:LastOutput = @("not found: $Exe")
        return 127
    }
    $previous = $ErrorActionPreference
    # Windows PowerShell 5.1 turns native stderr lines into errors; 'Stop' would abort on them.
    $ErrorActionPreference = 'Continue'
    try {
        & $Exe @Arguments 2>&1 | ForEach-Object {
            $text = [string]$_
            $lines.Add($text)
            Write-LogRaw $text
        }
        $code = $LASTEXITCODE
    } catch {
        $lines.Add([string]$_)
        Write-LogRaw ([string]$_)
        $code = 127
    } finally {
        $ErrorActionPreference = $previous
    }
    if ($null -eq $code) { $code = 0 }
    Write-Log "exit status $code"
    $script:LastOutput = $lines.ToArray()
    return [int]$code
}

# True when we may ask a question: a console user, input not redirected, not CI.
function Test-Interactive {
    if ($env:CI -or $env:IW4DOOM_NONINTERACTIVE) { return $false }
    if (-not [Environment]::UserInteractive) { return $false }
    try { if ([Console]::IsInputRedirected) { return $false } } catch { return $false }
    return $true
}
# Asks "QUESTION [Y/n]"; true on yes (Enter counts as yes).
function Confirm-Yes([string]$Question) {
    $answer = Read-Host ('      ' + $Question + ' [Y/n]')
    Write-Log ("      $Question [Y/n] -> '$answer'")
    return ($answer -eq '' -or $answer -match '^(y|yes)$')
}

function Test-Iwad([string]$Path) {
    try {
        $fs = [IO.File]::OpenRead($Path)
        try {
            $buf = New-Object byte[] 4
            $n = $fs.Read($buf, 0, 4)
            return ($n -eq 4 -and [Text.Encoding]::ASCII.GetString($buf) -eq 'IWAD')
        } finally {
            $fs.Dispose()
        }
    } catch {
        return $false
    }
}
# Full paths of the IWADs in a folder.
function Get-Iwads([string]$Dir) {
    if (-not (Test-Path -LiteralPath $Dir -PathType Container)) { return @() }
    $found = @(Get-ChildItem -LiteralPath $Dir -File -ErrorAction SilentlyContinue |
        Where-Object { Test-Iwad $_.FullName } | ForEach-Object { $_.FullName })
    return $found
}
# The preferred IWAD by $IwadOrder (names compare case-insensitively), then any other.
function Select-Iwad([string]$Dir) {
    $found = @(Get-Iwads $Dir)
    foreach ($want in $IwadOrder) {
        foreach ($f in $found) {
            if ((Split-Path -Leaf $f).ToLowerInvariant() -eq $want) { return $f }
        }
    }
    if ($found.Count -gt 0) { return $found[0] }
    return ''
}

# Steam library folders (from the registry and libraryfolders.vdf), existing ones only.
function Get-SteamLibraries {
    $libs = New-Object System.Collections.Generic.List[string]
    $steam = $null
    try {
        $steam = (Get-ItemProperty -Path 'HKCU:\Software\Valve\Steam' -Name SteamPath -ErrorAction Stop).SteamPath
    } catch {
        $steam = $null
    }
    if (-not $steam) { $steam = Join-Path ${env:ProgramFiles(x86)} 'Steam' }
    $steam = $steam -replace '/', '\'
    if (Test-Path -LiteralPath $steam -PathType Container) { $libs.Add($steam) }
    $vdf = Join-Path $steam 'steamapps\libraryfolders.vdf'
    if (Test-Path -LiteralPath $vdf -PathType Leaf) {
        foreach ($line in (Get-Content -LiteralPath $vdf -ErrorAction SilentlyContinue)) {
            if ($line -match '^\s*"path"\s+"(.+)"\s*$') {
                $p = $Matches[1] -replace '\\\\', '\'
                if ((Test-Path -LiteralPath $p -PathType Container) -and -not $libs.Contains($p)) { $libs.Add($p) }
            }
        }
    }
    return $libs.ToArray()
}
# Where Steam installed MW2 multiplayer, or ''.
function Find-SteamMw2 {
    foreach ($lib in (Get-SteamLibraries)) {
        $p = Join-Path $lib ('steamapps\common\' + $Mw2SteamFolder)
        if (Test-Path -LiteralPath (Join-Path $p 'zone') -PathType Container) { return $p }
    }
    return ''
}
# IWADs inside Steam's DOOM installs (Ultimate Doom, Doom 2, Final Doom), at most 8.
function Find-SteamIwads {
    $out = New-Object System.Collections.Generic.List[string]
    foreach ($lib in (Get-SteamLibraries)) {
        foreach ($game in @('Ultimate Doom', 'Doom 2', 'Final Doom')) {
            $p = Join-Path $lib ('steamapps\common\' + $game)
            if (-not (Test-Path -LiteralPath $p -PathType Container)) { continue }
            Get-ChildItem -LiteralPath $p -Recurse -File -Filter '*.wad' -ErrorAction SilentlyContinue |
                Where-Object { $_.Length -gt 1MB -and (Test-Iwad $_.FullName) } |
                ForEach-Object { if ($out.Count -lt 8) { $out.Add($_.FullName) } }
        }
    }
    return $out.ToArray()
}

function Get-Stamp([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return 'none' }
    $item = Get-Item -LiteralPath $Path
    return ('{0} {1}' -f $item.LastWriteTimeUtc.Ticks, $item.Length)
}

# ---------------------------------------------------------------------------------------------
# --list
# ---------------------------------------------------------------------------------------------
$DoomDir = Join-Path $GamesDir 'doom'
if ($List) {
    $default = Select-Iwad $DoomDir
    if (-not $default) {
        Write-Host ('No IWADs in ' + (Show-Path $DoomDir) + '\ (see games\README.md)')
        exit 1
    }
    Write-Host ('IWADs in ' + (Show-Path $DoomDir) + '\:')
    foreach ($f in (Get-Iwads $DoomDir)) {
        if ($f -eq $default) { Write-Host ('  ' + (Split-Path -Leaf $f) + '  (default)') }
        else { Write-Host ('  ' + (Split-Path -Leaf $f)) }
    }
    exit 0
}

[IO.File]::WriteAllText($Log, '')
Write-Log "IW4DOOM in $Root, games folder $GamesDir"

# ---------------------------------------------------------------------------------------------
# 1. Inputs
# ---------------------------------------------------------------------------------------------
Step 1 ('Checking your game files in ' + (Show-Path $GamesDir) + '\')
$problems = New-Object System.Collections.Generic.List[string]
$Iw4 = Join-Path $GamesDir 'iw4'
$Zone = Join-Path $Iw4 'zone'
$mw2Missing = $false
if (-not (Test-Path -LiteralPath $Iw4 -PathType Container)) {
    $mw2Missing = $true
    $problems.Add('missing: ' + (Show-Path $Iw4) + '\ - your Modern Warfare 2 (2009) MULTIPLAYER install: a junction, symlink or copy of the folder with iw4mp.exe, main\ and zone\')
} elseif (-not (Test-Path -LiteralPath $Zone -PathType Container)) {
    $mw2Missing = $true
    $problems.Add((Show-Path $Iw4) + '\ has no zone\ folder - point it at the MW2 install folder itself (the one with iw4mp.exe). A symlink made without Developer Mode does not work; use a junction (below).')
} else {
    $common = Get-ChildItem -LiteralPath $Zone -Recurse -File -Filter 'common_mp.ff' -ErrorAction SilentlyContinue | Select-Object -First 1
    $zoneFf = Get-ChildItem -LiteralPath $Zone -Recurse -File -Filter ($ExportZone + '.ff') -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $common) {
        $problems.Add((Show-Path $Iw4) + '\zone has no common_mp.ff - the multiplayer install is needed (single-player only has common.ff)')
    } elseif (-not $zoneFf) {
        $problems.Add((Show-Path $Iw4) + "\zone has no $ExportZone.ff - the exporter loads that multiplayer map; check the install is complete")
    } else {
        Write-Log ('MW2: ' + $common.FullName + ', ' + $zoneFf.FullName)
    }
}
$Iwad = ''
if ($IwadArg) {
    $candidate = $IwadArg
    if (-not [IO.Path]::IsPathRooted($candidate)) { $candidate = Join-Path $CallerPwd $candidate }
    if (Test-Path -LiteralPath $candidate -PathType Leaf) {
        $Iwad = [IO.Path]::GetFullPath($candidate)
    } else {
        foreach ($f in (Get-Iwads $DoomDir)) {
            $base = (Split-Path -Leaf $f).ToLowerInvariant()
            $wantName = $IwadArg.ToLowerInvariant()
            if ($base -eq $wantName -or $base -eq ($wantName + '.wad')) { $Iwad = $f; break }
        }
    }
    if (-not $Iwad) {
        $problems.Add("no IWAD '$IwadArg' in " + (Show-Path $DoomDir) + '\ (iw4doom.bat --list shows what is there)')
    }
} else {
    $Iwad = Select-Iwad $DoomDir
    if (-not $Iwad) {
        $problems.Add('missing: a Doom IWAD in ' + (Show-Path $DoomDir) + "\ - doom2.wad, doom.wad, plutonia.wad, tnt.wad, doom1.wad (shareware) or Freedoom's freedoom1.wad / freedoom2.wad")
    }
}
if ($problems.Count -gt 0) {
    $lines = New-Object System.Collections.Generic.List[string]
    $lines.AddRange($problems)
    if ($mw2Missing) {
        $mw2 = Find-SteamMw2
        $target = $mw2
        if (-not $target) { $target = 'C:\Program Files (x86)\Steam\steamapps\common\' + $Mw2SteamFolder }
        if ($mw2) { $lines.Add('Steam has MW2 at: ' + $mw2) }
        $lines.Add('Link it into games\ with a junction (no admin rights needed), from this folder:')
        $lines.Add('  cmd /c mklink /J "' + $Iw4 + '" "' + $target + '"')
    }
    if (-not $Iwad -and -not $IwadArg) {
        $steamIwads = @(Find-SteamIwads)
        if ($steamIwads.Count -gt 0) {
            $lines.Add('Steam has these IWADs; copy one into ' + (Show-Path $DoomDir) + '\, for example:')
            $lines.Add('  mkdir "' + $DoomDir + '"')
            foreach ($w in $steamIwads) { $lines.Add('  copy "' + $w + '" "' + $DoomDir + '\"') }
        }
    }
    $lines.Add('See ' + (Show-Path (Join-Path $Root 'games\README.md')) + ' for the folder layout.')
    Fail $lines.ToArray()
}
Info ('MW2 multiplayer install: ' + (Show-Path $Iw4))
$note = ''
if (-not $IwadArg -and @(Get-Iwads $DoomDir).Count -gt 1) {
    $note = ' (the preferred one; --list shows the others, --iwad NAME picks one)'
}
Info ('Doom IWAD: ' + (Show-Path $Iwad) + $note)
$Sf2 = Join-Path $GamesDir 'soundfont.sf2'
if (Test-Path -LiteralPath $Sf2 -PathType Leaf) {
    Info ('Soundfont: ' + (Show-Path $Sf2))
} else {
    $Sf2 = ''
    Info 'No soundfont.sf2: the game runs without music (optional, see games\README.md)'
}

# ---------------------------------------------------------------------------------------------
# 2. Rust
# ---------------------------------------------------------------------------------------------
Step 2 'Checking the Rust toolchain'
function Add-CargoBinToPath {
    $dirs = @()
    if ($env:CARGO_HOME) { $dirs += (Join-Path $env:CARGO_HOME 'bin') }
    if ($env:USERPROFILE) { $dirs += (Join-Path $env:USERPROFILE '.cargo\bin') }
    foreach ($d in $dirs) {
        if ((Test-Path -LiteralPath (Join-Path $d 'cargo.exe') -PathType Leaf) -and -not (($env:Path -split ';') -contains $d)) {
            $env:Path = $d + ';' + $env:Path
        }
    }
}
function Test-Command([string]$Name) {
    return [bool](Get-Command $Name -ErrorAction SilentlyContinue)
}
$winget = Test-Command 'winget'
if (-not (Test-Command 'cargo')) { Add-CargoBinToPath }
if (-not (Test-Command 'cargo') -and -not (Test-Command 'rustup')) {
    $manual = @(
        "Rust is not installed. Download and run rustup-init.exe from $RustupUrl",
        '(accept the defaults), then open a new terminal and run iw4doom.bat again.'
    )
    if ((Test-Interactive) -and $winget) {
        Info 'Rust is not installed. It can be installed now with winget (official rustup package):'
        Info '  winget install --id Rustlang.Rustup -e'
        if (Confirm-Yes 'Install Rust now?') {
            $code = Invoke-Logged 'winget' @('install', '--id', 'Rustlang.Rustup', '-e', '--accept-source-agreements', '--accept-package-agreements')
            if ($code -ne 0) { Fail (@("winget could not install Rust (exit $code).") + $manual) }
            Add-CargoBinToPath
            if (-not (Test-Command 'cargo')) {
                Fail @('Rust was installed; open a new terminal and run iw4doom.bat again.')
            }
            Info 'Rust installed.'
        } else {
            Fail $manual
        }
    } else {
        Fail $manual
    }
}
if (Test-Command 'rustup') {
    $channel = 'stable'
    $tc = Join-Path $Root 'rust-toolchain.toml'
    if (Test-Path -LiteralPath $tc) {
        foreach ($line in (Get-Content -LiteralPath $tc)) {
            if ($line -match '^channel\s*=\s*"([^"]+)"') { $channel = $Matches[1]; break }
        }
    }
    $code = Invoke-Logged 'rustup' @('run', $channel, 'rustc', '--version')
    if ($code -eq 0) {
        Write-Log "toolchain $channel already installed"
    } else {
        Info "Installing the Rust $channel toolchain (rust-toolchain.toml)..."
        $code = Invoke-Logged 'rustup' @('toolchain', 'install', $channel, '--profile', 'minimal', '-c', 'rustfmt', '-c', 'clippy')
        if ($code -ne 0) { Fail @("rustup could not install the $channel toolchain.") }
    }
} elseif (-not (Test-Command 'cargo')) {
    Fail @('rustup is installed but cargo is not on PATH; open a new terminal and run iw4doom.bat again.')
}
$code = Invoke-Logged 'rustc' @('--version')
$rustcVersion = ''
if ($code -eq 0) {
    # rustup may print its own info lines first.
    foreach ($line in $script:LastOutput) {
        if ($line -match '^rustc (\d+\.\d+\.\d+)') { $rustcVersion = $Matches[1]; break }
    }
}
if (-not $rustcVersion) { Fail @("rustc does not run. Reinstall Rust from $RustupUrl") }
if ([version]$rustcVersion -lt $MinRust) {
    Fail @("rustc $rustcVersion is too old; IW4DOOM needs $MinRust or newer. Update with:", '  rustup update stable')
}
Info "rustc $rustcVersion"

# ---------------------------------------------------------------------------------------------
# 3. C++ build tools (the MSVC linker)
# ---------------------------------------------------------------------------------------------
Step 3 'Checking the Visual Studio C++ build tools'
function Test-Msvc {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (Test-Path -LiteralPath $vswhere -PathType Leaf) {
        $previous = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        try {
            $path = & $vswhere -latest -products '*' -requires 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64' -property installationPath 2>$null
        } finally {
            $ErrorActionPreference = $previous
        }
        if ($path) { return [string]$path }
    }
    $link = Get-Command 'link.exe' -ErrorAction SilentlyContinue
    if ($link -and $link.Path -notmatch '\\Git\\') { return (Split-Path -Parent $link.Path) }
    return ''
}
$msvc = Test-Msvc
if (-not $msvc) {
    $manual = @(
        'The Visual Studio C++ build tools are missing (Rust needs their linker).',
        "Install Build Tools for Visual Studio from $BuildToolsUrl",
        'and tick "Desktop development with C++", then run iw4doom.bat again.'
    )
    $wingetArgs = @('install', '--id', 'Microsoft.VisualStudio.2022.BuildTools', '-e', '--override',
        '--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended')
    if ((Test-Interactive) -and $winget) {
        Info 'The Visual Studio C++ build tools are missing (Rust needs their linker).'
        Info 'They can be installed now with winget (about 2-4 GB; Windows asks for permission):'
        Info '  winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"'
        if (Confirm-Yes 'Install now?') {
            $code = Invoke-Logged 'winget' ($wingetArgs + @('--accept-source-agreements', '--accept-package-agreements'))
            if ($code -ne 0) { Fail (@("winget could not install the build tools (exit $code).") + $manual) }
            $msvc = Test-Msvc
            if (-not $msvc) { Fail @('The build tools were installed; open a new terminal and run iw4doom.bat again.') }
        } else {
            Fail $manual
        }
    } else {
        Fail $manual
    }
}
Info ('C++ build tools: ' + $msvc)

if ($Clean) {
    Info '--clean: removing previous build output'
    [void](Invoke-Logged 'cargo' @('clean'))
    [void](Invoke-Logged 'cargo' @('clean', '--manifest-path', 'tools\dmw_export\Cargo.toml'))
}

# ---------------------------------------------------------------------------------------------
# 4. Build the exporter
# ---------------------------------------------------------------------------------------------
$Exporter = Join-Path $Root 'tools\dmw_export\target\release\dmw_export.exe'
if (Test-Path -LiteralPath $Exporter -PathType Leaf) {
    Step 4 'Building the exporter (up to date unless the code changed)'
} else {
    Step 4 'Building the exporter (tools\dmw_export; the first build takes about 10-15 minutes)'
}
$code = Invoke-Logged 'cargo' @('build', '--release', '--locked', '--manifest-path', 'tools\dmw_export\Cargo.toml')
if ($code -ne 0) { Fail @('The exporter did not build. The compiler output is in ' + (Show-Path $Log) + '.') }

# ---------------------------------------------------------------------------------------------
# 5. Export the pack
# ---------------------------------------------------------------------------------------------
$Pack = Join-Path $Root 'packs\mw2'
$Manifest = Join-Path $Pack 'manifest.json'
function Get-JsonString($Object, [string]$Name) {
    $p = $Object.PSObject.Properties[$Name]
    if ($p) { return [string]$p.Value }
    return ''
}
[void](Invoke-Logged $Exporter @('--version'))
$wantGenerator = ''
if ($script:LastOutput.Count -gt 0) { $wantGenerator = $script:LastOutput[0] }
$wantRev = (Get-Content -LiteralPath (Join-Path $Root 'IW4L_REV') -Raw).Trim().Substring(0, 7)
$reason = ''
if ($Reexport) {
    $reason = '--reexport'
} elseif (-not (Test-Path -LiteralPath $Manifest -PathType Leaf)) {
    $reason = 'no pack yet'
} else {
    $m = $null
    try { $m = Get-Content -LiteralPath $Manifest -Raw | ConvertFrom-Json } catch { $m = $null }
    if ($null -eq $m) {
        $reason = 'manifest.json is unreadable'
    } else {
        $haveGenerator = Get-JsonString $m 'generator'
        $haveRev = Get-JsonString $m 'iw4l_rev'
        if ($haveGenerator -ne $wantGenerator) {
            $reason = "pack made by '$haveGenerator', exporter is '$wantGenerator'"
        } elseif ($haveRev -ne $wantRev) {
            $reason = "pack made with iw4L $haveRev, exporter uses $wantRev"
        }
    }
}
if (-not $reason) {
    Step 5 'Exporting your MW2 weapons to packs\mw2 (up to date; --reexport to redo)'
} else {
    Step 5 "Exporting your MW2 weapons to packs\mw2 ($reason; about a minute)"
    $env:DMW_MW2_GAMES = $GamesDir
    $code = Invoke-Logged $Exporter @('--out', $Pack, '--zone', $ExportZone)
    switch ($code) {
        0 {
            foreach ($line in $script:LastOutput) { if ($line -match '^(ok|FAILED) ') { Info $line } }
        }
        2 {
            Fail @(('Export failed: MW2 multiplayer was not found at ' + (Show-Path $Iw4) + '.'),
                'Check that it is the install folder (with iw4mp.exe, main\ and zone\).')
        }
        3 {
            Fail @('Export failed: iw4L refused to load the game files (exit 3).',
                ('The exporter output is in ' + (Show-Path $Log) + "; iw4L's own logs, if any, are under"),
                ((Show-Path (Join-Path $Root 'packs\.dmw_export-cache\iw4l-artifacts\logs')) + '\.'))
        }
        4 {
            $errs = @($script:LastOutput | Where-Object { $_ -match '^error: ' } | ForEach-Object { '  ' + ($_ -replace '^error: ', '') })
            Fail (@('Export failed: a weapon could not be exported:') + $errs + @('The previous pack (if any) is unchanged.'))
        }
        5 {
            Fail @(('Export failed: the pack could not be written to ' + (Show-Path $Pack) + ' (disk full or no permission?).'),
                'The previous pack (if any) is unchanged.')
        }
        default { Fail @("Export failed with exit status $code.") }
    }
}

# ---------------------------------------------------------------------------------------------
# 6. Build the game
# ---------------------------------------------------------------------------------------------
$Game = Join-Path $Root 'target\release\iw4doom.exe'
if (Test-Path -LiteralPath $Game -PathType Leaf) {
    Step 6 'Building the game (up to date unless the code changed)'
} else {
    Step 6 'Building the game (the first build takes a few minutes)'
}
$code = Invoke-Logged 'cargo' @('build', '--release', '--locked', '--bin', 'iw4doom')
if ($code -ne 0) { Fail @('The game did not build. The compiler output is in ' + (Show-Path $Log) + '.') }

# ---------------------------------------------------------------------------------------------
# 7. Smoke test (only when the game or the pack changed since the last passing one)
# ---------------------------------------------------------------------------------------------
$SmokeStamp = Join-Path $Root 'target\release\.iw4doom-smoke-ok'
$smokeKey = 'game ' + (Get-Stamp $Game) + '; pack ' + (Get-Stamp $Manifest)
$stampOk = $false
if (Test-Path -LiteralPath $SmokeStamp -PathType Leaf) {
    $stampOk = ((Get-Content -LiteralPath $SmokeStamp -Raw).Trim() -eq $smokeKey)
}
if ($stampOk) {
    Step 7 'Smoke test (passed before; the game and the pack are unchanged)'
} else {
    Step 7 ('Smoke test: 70 tics headless on ' + (Split-Path -Leaf $Iwad) + ' with your pack')
    $env:DMW_SMOKE_IWAD = $Iwad
    $env:DMW_SMOKE_PACK = $Pack
    $code = Invoke-Logged 'cargo' @('test', '--release', '--locked', '-p', 'room', '--test', 'setup_smoke', '--', '--nocapture')
    $okLine = @($script:LastOutput | Where-Object { $_ -match '^smoke: ok' } | Select-Object -First 1)
    if ($code -ne 0 -or $okLine.Count -eq 0) {
        if (Test-Path -LiteralPath $SmokeStamp) { Remove-Item -LiteralPath $SmokeStamp -Force }
        $why = @($script:LastOutput | Where-Object { $_ -match '^smoke: ' } | Select-Object -First 3)
        if ($why.Count -eq 0) { $why = @('the smoke test did not run (build error?)') }
        Fail (@('Smoke test failed: MW2 mode did not turn on.') + $why)
    }
    Info ($okLine[0] -replace '^smoke: ok: ', '')
    [IO.File]::WriteAllText($SmokeStamp, $smokeKey)
}

# ---------------------------------------------------------------------------------------------
# 8. Start the game
# ---------------------------------------------------------------------------------------------
# -mb 128: works around the engine's Z_FreeTags 2000-block limit (sheet 13 section 11).
$launch = New-Object System.Collections.Generic.List[string]
$launch.AddRange([string[]]@('-iwad', $Iwad, '-mw2pack', $Pack, '-mb', '128'))
if ($Sf2) { $launch.AddRange([string[]]@('-sf2', $Sf2)) }
$launch.AddRange($GameArgs)
Write-Log ('launch: "' + $Game + '" ' + (($launch | ForEach-Object { '"' + $_ + '"' }) -join ' '))
if ($SetupOnly) {
    Step 8 'Ready (--setup-only: not starting the game)'
    Info 'Start it with: iw4doom.bat'
    Write-Log 'done (setup only)'
    exit 0
}
Step 8 'Starting IW4DOOM'
Write-Log 'done'
$launchArgs = $launch.ToArray()
& $Game @launchArgs
exit $LASTEXITCODE
