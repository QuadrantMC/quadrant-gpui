# Recreates a throwaway Quadrant data folder and Minecraft folder with two
# fixture modpacks, so the app can be run without touching real data:
#   cargo run -- --data-dir "$env:TEMP\quadrant-gpui-sandbox\data" --mc-folder "$env:TEMP\quadrant-gpui-sandbox\mc"
param([string]$Root = (Join-Path $env:TEMP "quadrant-gpui-sandbox"))

$ErrorActionPreference = "Stop"
if (Test-Path $Root) { Remove-Item -Recurse -Force $Root }
$modpacks = Join-Path $Root "mc\modpacks"
New-Item -ItemType Directory -Force (Join-Path $Root "data") | Out-Null

function New-FixtureModpack([string]$Name, [string]$Version, [string]$Loader, [string[]]$Mods) {
    $dir = Join-Path $modpacks $Name
    New-Item -ItemType Directory -Force $dir | Out-Null
    # Each mod has a name, so listing never asks a provider to fill it in, and
    # a jar per manifest entry, so the pack does not count as having unknown files.
    $entries = foreach ($mod in $Mods) {
        Set-Content -Path (Join-Path $dir "$mod.jar") -Value "fixture"
        [ordered]@{
            name = $mod; id = "fixture-$mod"; downloadCount = 0; version = "1.0.0"
            modType = "Mod"; source = "ModSource.modRinth"; slug = $mod
            thumbnailUrls = @(); description = ""; license = ""; modIconUrl = ""
            downloadUrl = "https://cdn.modrinth.com/data/fixture/$mod.jar"
        }
    }
    $manifest = [ordered]@{
        modConfigVersion = "2"; quadrantVersion = "fixture"; name = $Name
        version = $Version; modLoader = $Loader; mods = @($entries)
    }
    $json = ConvertTo-Json $manifest -Depth 5
    [IO.File]::WriteAllText((Join-Path $dir "modConfigV2.json"), $json)
}

New-FixtureModpack "Alpha Pack" "1.21.1" "Fabric" @("sodium", "lithium", "iris")
New-FixtureModpack "Beta Pack" "1.20.1" "Forge" @("jei")
Get-ChildItem -Recurse $Root | Select-Object -ExpandProperty FullName
