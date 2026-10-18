# The frame-time harness in one command: builds a release binary with the
# `perf` feature, rebuilds the sandbox, runs the scenarios in a visible,
# maximized window and prints the median of each statistic over the runs.
# Results land in perf-log\runs\<time>-<sandbox> (JSON per scenario and
# summary.txt). Keep the mouse and keyboard off the window while it runs: the
# window ignores real input, but counts it and warns.
#   scripts\perf.ps1                       # every scenario, 5 runs, large sandbox
#   scripts\perf.ps1 -Scenario scroll-modpacks -Runs 3 -Sandbox small
param(
    [string]$Scenario = "all",
    [int]$Runs = 5,
    [ValidateSet("large", "small")][string]$Sandbox = "large",
    # offscreen: full size past the edge of the desktop, never focused or seen.
    # visible: maximized and focused; only for a confirmation run.
    [ValidateSet("offscreen", "visible")][string]$Placement = "offscreen",
    [string]$Out
)

$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot
if (-not $Out) {
    $Out = Join-Path $repo ("perf-log\runs\{0:yyyyMMdd-HHmmss}-{1}-{2}" -f (Get-Date), $Sandbox, $Scenario)
}

cargo build --release --locked --features perf --manifest-path (Join-Path $repo "Cargo.toml")
if ($LASTEXITCODE -ne 0) { throw "build failed" }

$root = Join-Path $env:TEMP ($(if ($Sandbox -eq "large") { "quadrant-gpui-sandbox-large" } else { "quadrant-gpui-sandbox" }))
& (Join-Path $PSScriptRoot "sandbox.ps1") -Large:($Sandbox -eq "large") -Root $root | Out-Null

$exe = Join-Path $repo "target\release\quadrant-native.exe"
& $exe --data-dir (Join-Path $root "data") --mc-folder (Join-Path $root "mc") `
    --perf $Scenario --perf-runs $Runs --perf-placement $Placement --perf-out $Out 2>&1 |
    Where-Object { "$_" -notmatch "^\[\*\]" } | ForEach-Object { "$_" }
if ($LASTEXITCODE -ne 0) { throw "perf run failed" }

""
"$Sandbox sandbox, median of $Runs runs ($Out):"
Get-Content (Join-Path $Out "summary.txt")
