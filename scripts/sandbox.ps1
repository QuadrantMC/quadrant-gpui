# Recreates a throwaway Quadrant data folder and Minecraft folder with two
# fixture modpacks and installed packs, so the app can be run without touching
# real data:
#   cargo run -- --data-dir "$env:TEMP\quadrant-gpui-sandbox\data" --mc-folder "$env:TEMP\quadrant-gpui-sandbox\mc"
# -Large adds a heavy library for perf runs, under quadrant-gpui-sandbox-large
# unless -Root says otherwise: 300 more modpacks, 500 mods in the applied one,
# and about 1000 resource and shader packs across the Minecraft folder and
# three Prism Launcher instances. The same seed gives the same folders.
param(
    [switch]$Large,
    [string]$Root = (Join-Path $env:TEMP ($(if ($Large) { "quadrant-gpui-sandbox-large" } else { "quadrant-gpui-sandbox" })))
)

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
        [IO.File]::WriteAllText((Join-Path $dir "$mod.jar"), "fixture")
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

$alphaMods = @("sodium", "lithium", "iris")
$random = [Random]::new(42)
$slugs = @("sodium", "lithium", "iris", "create", "jei", "waystones", "appleskin", "jade", "terralith", "farmers-delight",
    "modmenu", "cloth-config", "architectury", "balm", "ferrite-core", "entityculling", "continuity", "indium",
    "supplementaries", "moonlight", "geckolib", "curios", "botania", "mekanism", "ae2", "storage-drawers",
    "quark", "zeta", "xaeros-minimap", "journeymap", "travelers-backpack", "sophisticated-backpacks")
function New-ModList([int]$Count, [string]$Tag) {
    foreach ($i in 1..$Count) { "{0}-{1}{2:d3}" -f $slugs[$random.Next($slugs.Count)], $Tag, $i }
}
if ($Large) { $alphaMods = New-ModList 500 "a" }
New-FixtureModpack "Alpha Pack" "1.21.1" "Fabric" $alphaMods
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
        [IO.File]::SetLastWriteTime($path, [datetime]"2026-09-19 14:46")
    }
}
New-FixtureFiles (Join-Path $Root "mc\resourcepacks") @{ "FA+All_Extensions-v1.9.2.zip" = 380621; "FreshAnimations_v1.10.5.zip" = 645632 }
New-FixtureFiles (Join-Path $Root "mc\shaderpacks") @{ "Bliss_v2.1.2.zip" = 1782579; "ComplementaryReimagined_r5.5.zip" = 912000 }
function New-FixtureInstance([string]$Name) {
    $dir = Join-Path $Root "prism\instances\$Name"
    New-Item -ItemType Directory -Force $dir | Out-Null
    Set-Content -Path (Join-Path $dir "instance.cfg") -Value "[General]`nname=$Name"
    [IO.File]::WriteAllText((Join-Path $dir "mmc-pack.json"), '{"formatVersion":1,"components":[{"uid":"net.minecraft","version":"1.21.1"}]}')
    $dir
}
$instance = New-FixtureInstance "Survival"
New-FixtureFiles (Join-Path $instance "minecraft\resourcepacks") @{ "FreshAnimations_v1.10.5.zip" = 645632 }

if ($Large) {
    $loaders = @("Fabric", "Forge", "NeoForge", "Quilt")
    $versions = @("1.21.1", "1.20.1", "1.20.4", "1.19.2", "1.18.2")
    foreach ($i in 1..300) {
        $loader = $loaders[$random.Next($loaders.Count)]
        $version = $versions[$random.Next($versions.Count)]
        New-FixtureModpack ("Library Pack {0:d3}" -f $i) $version $loader (New-ModList ($random.Next(10, 150)) "p$i-")
    }
    # The pages list a pack's name, size and date, never its contents.
    function New-PackFiles([string]$Dir, [string]$Prefix, [int]$Count) {
        $files = @{}
        foreach ($i in 1..$Count) { $files["{0}_{1:d4}_v1.{2}.zip" -f $Prefix, $i, $random.Next(20)] = $random.Next(1024, 65536) }
        New-FixtureFiles $Dir $files
    }
    New-PackFiles (Join-Path $Root "mc\resourcepacks") "Resources" 400
    New-PackFiles (Join-Path $Root "mc\shaderpacks") "Shaders" 150
    foreach ($name in @("Survival", "Creative", "Skyblock")) {
        $dir = New-FixtureInstance $name
        New-PackFiles (Join-Path $dir "minecraft\resourcepacks") "$name-Resources" 120
        New-PackFiles (Join-Path $dir "minecraft\shaderpacks") "$name-Shaders" 30
    }
}
# Experimental features list Prism instances. Pointing the Prism Launcher
# folder into the sandbox keeps the host away from a real Prism install.
$config = [ordered]@{ experimentalFeatures = $true; prismLauncherFolder = (Join-Path $Root "prism") }
[IO.File]::WriteAllText((Join-Path $Root "data\config.json"), (ConvertTo-Json $config))
if ($Large) {
    $count = (Get-ChildItem -Recurse -File $Root | Measure-Object).Count
    "$Root ($count files)"
} else {
    Get-ChildItem -Recurse $Root | Select-Object -ExpandProperty FullName
}
