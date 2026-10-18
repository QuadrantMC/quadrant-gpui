# Recreates a throwaway Quadrant data folder and Minecraft folder with two
# fixture modpacks and installed packs, so the app can be run without touching
# real data:
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
# Applied the way Quadrant applies a pack: `mods` links to its folder. Needs
# Developer Mode (or admin) on Windows to create the link.
New-Item -ItemType SymbolicLink -Path (Join-Path $Root "mc\mods") -Target (Join-Path $modpacks "Alpha Pack") | Out-Null
# Installed content: packs in the Minecraft folder and in one Prism Launcher
# instance, so the Installed content page has two locations to copy between.
function New-FixtureFiles([string]$Dir, [hashtable]$Files) {
    New-Item -ItemType Directory -Force $Dir | Out-Null
    foreach ($name in $Files.Keys) {
        $path = Join-Path $Dir $name
        [IO.File]::WriteAllBytes($path, [byte[]]::new($Files[$name]))
        (Get-Item $path).LastWriteTime = [datetime]"2026-09-19 14:46"
    }
}
New-FixtureFiles (Join-Path $Root "mc\resourcepacks") @{ "FA+All_Extensions-v1.9.2.zip" = 380621; "FreshAnimations_v1.10.5.zip" = 645632 }
New-FixtureFiles (Join-Path $Root "mc\shaderpacks") @{ "Bliss_v2.1.2.zip" = 1782579; "ComplementaryReimagined_r5.5.zip" = 912000 }
$instance = Join-Path $Root "prism\instances\Survival"
New-FixtureFiles (Join-Path $instance "minecraft\resourcepacks") @{ "FreshAnimations_v1.10.5.zip" = 645632 }
Set-Content -Path (Join-Path $instance "instance.cfg") -Value "[General]`nname=Survival"
[IO.File]::WriteAllText((Join-Path $instance "mmc-pack.json"), '{"formatVersion":1,"components":[{"uid":"net.minecraft","version":"1.21.1"}]}')
# Experimental features list Prism instances. Pointing the Prism Launcher
# folder into the sandbox keeps the host away from a real Prism install.
$config = [ordered]@{ experimentalFeatures = $true; prismLauncherFolder = (Join-Path $Root "prism") }
[IO.File]::WriteAllText((Join-Path $Root "data\config.json"), (ConvertTo-Json $config))
Get-ChildItem -Recurse $Root | Select-Object -ExpandProperty FullName
