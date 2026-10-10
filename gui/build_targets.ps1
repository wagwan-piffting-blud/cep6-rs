# Build the Ceps GUI for every shipping target and collect the installers into
# one folder.
#
#     powershell -ExecutionPolicy Bypass -File build_targets.ps1
#     powershell -ExecutionPolicy Bypass -File build_targets.ps1 -Only linux-arm64
#     powershell -ExecutionPolicy Bypass -File build_targets.ps1 -Flavor voiceless
#     powershell -ExecutionPolicy Bypass -File build_targets.ps1 -Flavor both -Dist D:\drop
#     powershell -ExecutionPolicy Bypass -File build_targets.ps1 -Arm64Host ""   # arm64 under qemu, slow
#
# -Flavor full bundles the four voices (~440 MB per installer); voiceless
# ships none, and the app finds them in a folder the user picks, a Cepstral
# Swift install or $CEPS_VOICE_ROOT. Voiceless installers are named
# Ceps-voiceless_*, so both flavors can share a dist folder.
#
# Unlike the CLI, the GUI cannot be cross-built from here: Tauri links each
# platform's own webview (WebView2, webkitgtk, WKWebView). So each target
# builds where that webview lives:
#
#   windows-amd64    this machine, cargo tauri build      NSIS setup .exe + .msi
#   linux-amd64      Docker Desktop, linux/Dockerfile     .deb + .AppImage
#   linux-arm64      the Pi 5's Docker over ssh://         .deb + .AppImage
#   macos-universal  the Mac over ssh, build_macos.sh      signed, notarized .dmg
#
# The Linux builds send only ceps-rs (and, for full, the voices) as context
# and bring the bundles back with --output, so nothing has to be synced to the
# Pi. The Mac sees this folder over SMB and writes its .dmg straight into
# gui/dist; it asks for the login keychain password (codesign needs it
# unlocked).
#
# Run it from PowerShell, not cmd.

param(
    [string]$Only = "",
    [ValidateSet("full", "voiceless", "both")]
    [string]$Flavor = "full",
    [string]$Dist = "",
    [switch]$NoDist,
    [string]$Arm64Host = "wags@192.168.1.250",
    [string]$MacHost = "wags@192.168.1.86",
    [string]$MacGui = "/Volumes/LocalUser/Documents/ceps/ceps-rs/gui"
)

$ErrorActionPreference = "Stop"
$gui = $PSScriptRoot
$root = Split-Path (Split-Path $gui -Parent) -Parent
$guiDist = Join-Path $gui "dist"
if (-not $Dist) { $Dist = $guiDist }
if (-not (Test-Path $Dist)) { New-Item -ItemType Directory -Path $Dist | Out-Null }
$distFull = (Resolve-Path $Dist).Path

$flavors = if ($Flavor -eq "both") { @("full", "voiceless") } else { @($Flavor) }

$targets = @(
    @{ n = "windows-amd64";   note = "Windows 10/11 x64 (WebView2)" },
    @{ n = "linux-amd64";     note = "64-bit Linux PC, glibc 2.35+" },
    @{ n = "linux-arm64";     note = "Raspberry Pi 4/5, 64-bit OS, glibc 2.35+" },
    @{ n = "macos-universal"; note = "Apple silicon + Intel, macOS 11+" }
)

# Tauri names every bundle Ceps_<version>_...; a voiceless one becomes
# Ceps-voiceless_<version>_... on the way into dist.
function Collect($files) {
    foreach ($f in $files) {
        $name = $f.Name
        if ($script:fl -eq "voiceless" -and $name -like "Ceps_*") {
            $name = "Ceps-voiceless_" + $name.Substring(5)
        }
        $shown = $name
        if ($NoDist) {
            $shown = $f.FullName
        } else {
            Copy-Item $f.FullName (Join-Path $distFull $name) -Force
        }
        $script:built += [pscustomobject]@{
            Target = $script:cur; Flavor = $script:fl; Name = $shown; MB = [math]::Round($f.Length / 1MB)
        }
    }
}

function Build-Linux($arch, $dockerHost) {
    $tmp = Join-Path ([System.IO.Path]::GetTempPath()) "ceps-gui-$arch-$([guid]::NewGuid().ToString('N'))"
    $argv = @()
    if ($dockerHost) { $argv += @("-H", "ssh://$dockerHost") }
    $argv += @("build", "--platform", "linux/$arch", "--build-arg", "FLAVOR=$script:fl",
               "-f", (Join-Path $gui "linux/Dockerfile"),
               "--output", "type=local,dest=$tmp", $root)
    & docker @argv
    if ($LASTEXITCODE -ne 0) { throw "docker build for linux/$arch ($script:fl) failed" }
    Collect (Get-ChildItem $tmp -File)
    Remove-Item $tmp -Recurse -Force
}

$built = @()
foreach ($x in $targets) {
    if ($Only -and $x.n -ne $Only) { continue }
    foreach ($fl in $flavors) {
        $cur = $x.n
        Write-Host ""
        Write-Host "=== $($x.n), $fl " -NoNewline
        Write-Host "($($x.note))" -ForegroundColor DarkGray

        switch ($x.n) {
            "windows-amd64" {
                $bundle = Join-Path $gui "target/release/bundle"
                # stale installers of the other flavor must not be collected
                if (Test-Path $bundle) { Remove-Item $bundle -Recurse -Force }
                $argv = @("tauri", "build", "--bundles", "nsis,msi")
                if ($fl -eq "voiceless") { $argv += @("--config", "tauri.voiceless.json") }
                Push-Location $gui
                try {
                    & cargo @argv
                    if ($LASTEXITCODE -ne 0) { throw "windows-amd64 ($fl) failed" }
                } finally {
                    Pop-Location
                }
                $files = @(Get-ChildItem (Join-Path $bundle "nsis") -Filter "*.exe") +
                         @(Get-ChildItem (Join-Path $bundle "msi") -Filter "*.msi")
                Collect $files
            }
            "linux-amd64" {
                Build-Linux "amd64" ""
            }
            "linux-arm64" {
                if (-not $Arm64Host) {
                    Write-Host "  no -Arm64Host: building under qemu on Docker Desktop, expect hours" -ForegroundColor Yellow
                }
                Build-Linux "arm64" $Arm64Host
            }
            "macos-universal" {
                $envs = if ($fl -eq "voiceless") { "VOICELESS=1 " } else { "" }
                & ssh -t $MacHost "security unlock-keychain ~/Library/Keychains/login.keychain-db && cd $MacGui && $($envs)./build_macos.sh"
                if ($LASTEXITCODE -ne 0) { throw "macos-universal ($fl) failed" }
                # build_macos.sh names it itself and writes it into gui/dist over SMB
                $prefix = if ($fl -eq "voiceless") { "Ceps-voiceless_" } else { "Ceps_" }
                $dmg = Get-ChildItem $guiDist -Filter "$($prefix)*_universal.dmg" |
                    Sort-Object LastWriteTime | Select-Object -Last 1
                if ($guiDist -ne $distFull -and -not $NoDist) {
                    Copy-Item $dmg.FullName (Join-Path $distFull $dmg.Name) -Force
                }
                $built += [pscustomobject]@{
                    Target = $cur; Flavor = $fl; Name = $dmg.Name; MB = [math]::Round($dmg.Length / 1MB)
                }
            }
        }
    }
}

Write-Host ""
$built | Format-Table Target, Flavor, Name, MB -AutoSize
if (-not $NoDist) { Write-Host "collected into $distFull" -ForegroundColor Green }
