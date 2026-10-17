# Builds every plugin in the bundle and packs portable bundles into dist\.
# Install: copy dist\<Name>.vst3 into C:\Program Files\Common Files\VST3
# (FL Studio only scans VST3 there, not in custom VST folders).
param([string[]]$Only)
$ErrorActionPreference = 'Stop'

# crate name -> plugin file name
$plugins = [ordered]@{
    'petal-granulizer' = 'Petal Granulizer'
    'petal-eq' = 'Petal EQ'
    'petal-comp' = 'Petal Comp'
    'petal-squash' = 'Petal Squash'
    'petal-delay' = 'Petal Delay'
    'petal-verb' = 'Petal Verb'
    'petal-double' = 'Petal Double'
    'petal-retro' = 'Petal Retro'
    'petal-shift' = 'Petal Shift'
    'petal-crush' = 'Petal Crush'
    'petal-oracle' = 'Petal Oracle'
    'petal-mod' = 'Petal Mod'
    'petal-limit' = 'Petal Limit'
    'petal-half' = 'Petal Half'
    'petal-air' = 'Petal Air'
}

# Toolchain: by default the system `cargo` (rustup, any Windows toolchain).
# Optionally point PETAL_TOOLCHAIN at a portable folder with cargo\, rustup\
# and gcc\w64devkit\ (the windows-gnu setup this bundle was developed with).
$tools = if ($env:PETAL_TOOLCHAIN) { $env:PETAL_TOOLCHAIN } else { Join-Path $PSScriptRoot '..\.tools\rust-daw' }
if (Test-Path "$tools\cargo\bin\cargo.exe") {
    $env:CARGO_HOME = (Resolve-Path "$tools\cargo").Path
    $env:RUSTUP_HOME = (Resolve-Path "$tools\rustup").Path
    $env:PATH = "$env:CARGO_HOME\bin;$env:PATH"
    if (Test-Path "$tools\gcc\w64devkit\bin") {
        # w64devkit provides gcc, as and dlltool. Its libgcc.a already contains the SEH
        # unwinder, so the empty libgcc_eh.a in .cargo\libstub satisfies rustc.
        $env:PATH = (Resolve-Path "$tools\gcc\w64devkit\bin").Path + ";$env:PATH"
        $env:LIBRARY_PATH = (Resolve-Path "$PSScriptRoot\.cargo\libstub").Path
    }
}
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "cargo not found: install Rust from https://rustup.rs or set PETAL_TOOLCHAIN"
}

Push-Location $PSScriptRoot
try {
    foreach ($crate in $plugins.Keys) {
        if ($Only -and $Only -notcontains $crate) { continue }
        $name = $plugins[$crate]
        cargo build --release -p $crate --lib
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed for $crate" }

        $dll = "target\release\$($crate.Replace('-', '_')).dll"
        $bundle = "dist\$name.vst3"
        if (Test-Path $bundle) { Remove-Item -Recurse -Force $bundle }
        # VST3 bundle layout: Name.vst3\Contents\x86_64-win\Name.vst3
        New-Item -ItemType Directory -Force "$bundle\Contents\x86_64-win" | Out-Null
        Copy-Item $dll "$bundle\Contents\x86_64-win\$name.vst3"
        Copy-Item $dll "dist\$name.clap" -Force
        Write-Host "Packed $name"
    }
    Write-Host "Done: $((Resolve-Path dist).Path)"
}
finally {
    Pop-Location
}
