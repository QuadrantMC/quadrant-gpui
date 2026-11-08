# Tests the whole self-update on Windows without production: a throwaway
# minisign key, two debug builds of HEAD (the committed version and the next
# one) packaged as NSIS setups, and a local update server. It installs the
# old build for this user, lets it find, download and verify the new one,
# presses "Install an update" through UI Automation, and checks that the new
# version replaced it and started. Then it serves a tampered setup and one
# signed with another key and checks that both are refused, uninstalls, and
# checks nothing is left. The app runs against a sandbox, never real data.
#
# Needs cargo-packager and the Tauri CLI (`../quadrant/node_modules`, after
# `bun install` there). Refuses to run if Quadrant-Native is installed.
param(
    [int]$Port = 8765,
    [int]$TimeoutSeconds = 180,
    [switch]$InChild
)

$ErrorActionPreference = "Stop"
$repo = Split-Path $PSScriptRoot
$work = Join-Path $env:TEMP "quadrant-native-update-e2e"
if (-not $InChild) {
    # The test sets the throwaway update key, the signing keys, the cargo
    # target and the sandbox folders as environment variables. Run in the
    # caller's shell they would outlive the script, and the next release
    # build from that shell would trust only the throwaway key. A child
    # process takes them with it. Its private key goes with it too.
    try {
        & (Get-Process -Id $PID).Path -NoProfile -File $PSCommandPath -Port $Port -TimeoutSeconds $TimeoutSeconds -InChild
        $code = $LASTEXITCODE
    } finally {
        Remove-Item -Recurse -Force "$work\keys" -ErrorAction SilentlyContinue
    }
    exit $code
}
$installDir = Join-Path $env:LOCALAPPDATA "Quadrant-Native"
$uninstallKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Quadrant-Native"
$startMenu = Join-Path ([Environment]::GetFolderPath("Programs")) "Quadrant-Native"
$tauri = Join-Path $repo "..\quadrant\node_modules\.bin\tauri.exe"

function Step([string]$Text) { Write-Host "== $Text" -ForegroundColor Cyan }
function Check([bool]$Ok, [string]$Text) {
    if (-not $Ok) { throw "FAILED: $Text" }
    Write-Host "ok: $Text" -ForegroundColor Green
}
function WaitFor([scriptblock]$Condition, [string]$Text) {
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while (-not (& $Condition)) {
        if ((Get-Date) -gt $deadline) { throw "timed out waiting for: $Text" }
        Start-Sleep -Milliseconds 500
    }
}
function Installed { Get-Process quadrant-native -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$installDir\*" } }
function FileVersion { (Get-Item (Join-Path $installDir "quadrant-native.exe")).VersionInfo.ProductVersion }

if ((Test-Path $installDir) -or (Test-Path $uninstallKey)) { throw "Quadrant-Native is installed for this user; uninstall it first" }
if (-not (Test-Path $tauri)) { throw "no Tauri CLI at $tauri; run bun install in ../quadrant" }
$tauriApp = Get-ItemProperty "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Quadrant" -ErrorAction SilentlyContinue
$tauriExe = Get-Item (Join-Path $env:LOCALAPPDATA "Quadrant\quadrant_next.exe") -ErrorAction SilentlyContinue

if (Test-Path $work) { Remove-Item -Recurse -Force $work }
New-Item -ItemType Directory -Force "$work\keys", "$work\site\files" | Out-Null

Step "throwaway signing keys"
& $tauri signer generate --ci -p "" -w "$work\keys\test.key" | Out-Null
& $tauri signer generate --ci -p "other" -w "$work\keys\other.key" | Out-Null
$env:QUADRANT_UPDATER_PUBKEY =(Get-Content -Raw "$work\keys\test.key.pub").Trim()

Step "building the old and the new version"
$manifest = Get-Content -Raw (Join-Path $repo "Cargo.toml")
$old = [regex]::Match($manifest, '(?m)^version = "(\d+)\.(\d+)\.(\d+)-([^"]+)"')
$oldVersion = "$($old.Groups[1]).$($old.Groups[2]).$($old.Groups[3])-$($old.Groups[4])"
$newVersion = "$($old.Groups[1]).$($old.Groups[2]).$([int]$old.Groups[3].Value + 1)-$($old.Groups[4])"
$env:CARGO_TARGET_DIR = "$work\target"
# Scoop's NSIS sets NSISDIR, which hides cargo-packager's NSIS plugins.
Remove-Item Env:NSISDIR -ErrorAction SilentlyContinue
$setups = @{}
foreach ($version in $oldVersion, $newVersion) {
    $src = "$work\src-$version"
    New-Item -ItemType Directory $src | Out-Null
    git -C $repo archive --format=tar HEAD | tar -x -C $src
    if ($LASTEXITCODE -ne 0) { throw "git archive failed" }
    (Get-Content -Raw "$src\Cargo.toml").Replace("version = `"$oldVersion`"", "version = `"$version`"") |
        Set-Content -NoNewline "$src\Cargo.toml"
    cargo build --features updater --manifest-path "$src\Cargo.toml"
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
    Push-Location $src
    cargo packager -f nsis --binaries-dir "$work\target\debug" --out-dir "$work\out-$version"
    Pop-Location
    $setup = "$work\out-$version\quadrant-native_${version}_x64-setup.exe"
    Check (Test-Path $setup) "built $setup"
    $setups[$version] = Copy-Item $setup "$work\$version-setup.exe" -PassThru
}

Step "signing the new setup"
$newSetup = $setups[$newVersion].FullName
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content -Raw "$work\keys\test.key"
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ""
& $tauri signer sign $newSetup | Out-Null
$signature = Get-Content -Raw "$newSetup.sig"
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content -Raw "$work\keys\other.key"
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = "other"
Copy-Item $newSetup "$work\other-key-setup.exe"
& $tauri signer sign "$work\other-key-setup.exe" | Out-Null
$otherSignature = Get-Content -Raw "$work\other-key-setup.exe.sig"
Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY, Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD
Copy-Item $newSetup "$work\site\files\good-setup.exe"
$bytes = [IO.File]::ReadAllBytes($newSetup)
$bytes[$bytes.Length - 100] = $bytes[$bytes.Length - 100] -bxor 0xFF
[IO.File]::WriteAllBytes("$work\site\files\tampered-setup.exe", $bytes)

# The update server's answer for a release, as the API serves it.
function Publish([string]$Version, [string]$File, [string]$Signature) {
    $release = [ordered]@{
        product = "quadrant-native"; version = $Version; pub_date = (Get-Date).ToString("o")
        arch = "x86_64"; platform = "windows"; platform_variation = "nsis"; branch = "gpui-preview"; public = $true
        url = "http://localhost:$Port/files/$File"; signature = $Signature.Trim()
    }
    $release | ConvertTo-Json | Set-Content "$work\site\release.json"
}

$server = Start-Job -ArgumentList "$work\site", $Port -ScriptBlock {
    param($root, $port)
    $listener = [System.Net.HttpListener]::new()
    $listener.Prefixes.Add("http://localhost:$port/")
    $listener.Start()
    while ($true) {
        $context = $listener.GetContext()
        $path = $context.Request.Url.AbsolutePath
        "$($context.Request.HttpMethod) $($context.Request.Url.PathAndQuery)"
        $file = if ($path -like "/api/any/quadrant-native/updates/*") { "$root\release.json" } else { Join-Path $root $path.TrimStart("/") }
        if (Test-Path $file -PathType Leaf) {
            $body = [IO.File]::ReadAllBytes($file)
            $context.Response.ContentLength64 = $body.Length
            $context.Response.OutputStream.Write($body, 0, $body.Length)
        } else {
            $context.Response.StatusCode = 404
        }
        $context.Response.Close()
    }
}

try {
    Step "installing $oldVersion for this user"
    Start-Process $setups[$oldVersion].FullName -ArgumentList "/S" -Wait
    Check ((FileVersion) -eq $oldVersion) "installed exe is $oldVersion"
    Check ((Get-ItemProperty $uninstallKey).DisplayVersion -eq $oldVersion) "uninstall entry says $oldVersion"
    Check (Test-Path "$startMenu\Quadrant-Native.lnk") "Start menu shortcut exists"

    & "$repo\scripts\sandbox.ps1" -Root "$work\sandbox" | Out-Null
    # A Quadrant-Native channel, or the update check offers to switch to Quadrant.
    $config = Get-Content -Raw "$work\sandbox\data\config.json" | ConvertFrom-Json
    $config | Add-Member -Force channel "gpui-preview"
    $config | ConvertTo-Json -Depth 10 | Set-Content "$work\sandbox\data\config.json"
    $env:QUADRANT_GPUI_DATA_DIR = "$work\sandbox\data"
    $env:QUADRANT_GPUI_MC_FOLDER = "$work\sandbox\mc"
    $env:QUADRANT_NATIVE_UPDATE_ENDPOINT = "http://localhost:$Port"
    # Windows without focus, which Hide moves off-screen, so the test
    # doesn't take over the screen. The setup's relaunch inherits both.
    $env:QUADRANT_NATIVE_BACKGROUND = "1"
    # A startup error goes to the log, never to a dialog on the desktop.
    $env:QUADRANT_NATIVE_NO_DIALOG = "1"
    $exe = Join-Path $installDir "quadrant-native.exe"

    Add-Type -Namespace E2E -Name Win32 -MemberDefinition @"
[DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hwnd, IntPtr after, int x, int y, int cx, int cy, uint flags);
"@
    function Hide($Process) {
        WaitFor { $Process.Refresh(); $Process.MainWindowHandle -ne 0 } "the window"
        # SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE
        [E2E.Win32]::SetWindowPos($Process.MainWindowHandle, 0, -30000, -30000, 0, 0, 0x15) | Out-Null
    }
    function Run([string]$Log) {
        $process = Start-Process $exe -WindowStyle Hidden -RedirectStandardError "$work\$Log" -RedirectStandardOutput "$work\$Log.out" -PassThru
        Hide $process
        $process
    }
    function LogHas([string]$Log, [string]$Text) {
        (Test-Path "$work\$Log") -and (Get-Content -Raw "$work\$Log") -like "*$Text*"
    }

    Step "updating $oldVersion to $newVersion"
    Publish $newVersion "good-setup.exe" $signature
    $app = Run "update.log"
    WaitFor { LogHas "update.log" "Quadrant-Native $newVersion is downloaded and its signature checks out" } "the verified download"
    Check $true "found, downloaded and verified $newVersion"
    Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
    $ui = [System.Windows.Automation.AutomationElement]
    $byName = [System.Windows.Automation.PropertyCondition]::new($ui::NameProperty, "Install an update")
    $button = $null
    WaitFor {
        $window = $ui::RootElement.FindFirst("Children", [System.Windows.Automation.PropertyCondition]::new($ui::ProcessIdProperty, $app.Id))
        $script:button = if ($window) { $window.FindFirst("Descendants", $byName) }
        $null -ne $script:button
    } "the Install an update button"
    Check $true "the title bar offers Install an update"
    $button.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
    WaitFor { $app.HasExited } "the old version to quit"
    WaitFor { (FileVersion) -eq $newVersion -and (Installed) } "the new version to start"
    Hide (Installed)
    Check ((FileVersion) -eq $newVersion) "installed exe is now $newVersion"
    Check ((Get-ItemProperty $uninstallKey).DisplayVersion -eq $newVersion) "uninstall entry says $newVersion"
    $relaunched = Installed
    Check ($relaunched.StartTime -gt $app.ExitTime) "the setup started the new version after the old one quit (pid $($relaunched.Id))"
    $relaunched | Stop-Process -Force
    WaitFor { -not (Installed) } "the relaunched app to stop"

    $nextVersion = "$($old.Groups[1]).$($old.Groups[2]).$([int]$old.Groups[3].Value + 2)-$($old.Groups[4])"
    foreach ($case in @(
            @{ Name = "tampered setup"; File = "tampered-setup.exe"; Signature = $signature; Log = "tampered.log" },
            @{ Name = "setup signed with another key"; File = "good-setup.exe"; Signature = $otherSignature; Log = "other-key.log" })) {
        Step "refusing a $($case.Name)"
        Publish $nextVersion $case.File $case.Signature
        $app = Run $case.Log
        WaitFor { LogHas $case.Log "Update failed" } "the refusal"
        Start-Sleep -Seconds 2
        $log = Get-Content -Raw "$work\$($case.Log)"
        Check ($log -like "*Downloading Quadrant-Native $nextVersion*") "downloaded the $($case.Name)"
        Check ($log -like "*signature doesn't match*" -and $log -notlike "*checks out*") "refused it: $(($log -split "`n" | Select-String 'Update failed').Line.Trim())"
        Check ((FileVersion) -eq $newVersion) "still $newVersion installed"
        $app | Stop-Process -Force
        WaitFor { $app.HasExited } "the app to stop"
    }
} finally {
    Get-Process quadrant-native -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$installDir\*" } | Stop-Process -Force
    Step "update server requests"
    Receive-Job $server | ForEach-Object { "  $_" }
    Stop-Job $server; Remove-Job $server
    if (Test-Path "$installDir\uninstall.exe") {
        Step "uninstalling"
        Start-Process "$installDir\uninstall.exe" -ArgumentList "/S" -Wait
        WaitFor { -not (Test-Path "$installDir\quadrant-native.exe") } "the uninstaller"
        Start-Sleep -Seconds 2
    }
}
Check (-not (Test-Path $installDir)) "install folder removed"
Check (-not (Test-Path $uninstallKey)) "uninstall entry removed"
Check (-not (Test-Path $startMenu)) "Start menu folder removed"
# cargo-packager's uninstaller, like Tauri's, keeps the install folder it
# offers on the next install. Nothing else may be left there.
$productKey = "HKCU:\Software\MrQuantumOFF (Demir Yerli)\Quadrant-Native"
if (Test-Path $productKey) {
    $values = (Get-Item $productKey).Property
    Check (@($values).Count -eq 1 -and $values[0] -eq "(default)") "only the remembered install folder is left in $productKey"
    Remove-Item $productKey
}
$tauriAfter = Get-ItemProperty "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Quadrant" -ErrorAction SilentlyContinue
Check ("$($tauriApp.DisplayVersion)" -eq "$($tauriAfter.DisplayVersion)") "the Tauri app's uninstall entry is unchanged ($($tauriAfter.DisplayVersion))"
$tauriExeAfter = Get-Item (Join-Path $env:LOCALAPPDATA "Quadrant\quadrant_next.exe") -ErrorAction SilentlyContinue
Check ("$($tauriExe.LastWriteTimeUtc)" -eq "$($tauriExeAfter.LastWriteTimeUtc)") "the Tauri app's exe is unchanged"
Write-Host "All checks passed. Logs and builds are in $work" -ForegroundColor Green
