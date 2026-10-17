<div align="center">

# 🌸 petalbundle

**15 pastel audio effects. A little colour for your sound.**

Granular textures, generative resonance, creative effects and mixing tools — with a shared visual language and live visualizers.

**English** · [Русский](README.ru.md)

![Plugins](https://img.shields.io/badge/effects-15-b4a0e5?style=flat-square)
![Platform](https://img.shields.io/badge/Windows-x64-96c6e8?style=flat-square)
![Formats](https://img.shields.io/badge/formats-VST3%20%2B%20CLAP-a0d5b7?style=flat-square)
![Built with Rust](https://img.shields.io/badge/built_with-Rust-f7b28e?style=flat-square)
[![License](https://img.shields.io/badge/license-GPL--3.0--or--later-e9a5c9?style=flat-square)](LICENSE)

[Explore the plugins](#plugins) · [Installation](#installation) · [Build from source](#build) · [Screenshots](docs/screenshots)

</div>

[![All 15 Petal plugins: pastel interfaces and live visualizers](docs/all-plugins.png)](docs/all-plugins.png)

## Meet Petal

Petal is a collection of audio effects for Windows, written in Rust with [nih-plug](https://github.com/robbert-vdh/nih-plug). Each plugin has its own character, from flower-shaped grains to a rain-pond reverb, while sharing the same soft palette and controls.

- **Shape a sound:** granular processing, a generative resonator, lo-fi colour and tempo-synced half-time.
- **Build space and movement:** delay, reverb, multi-tap modulation, a four-voice doubler and pitch shifting.
- **Finish a mix:** an eight-band EQ, compression, multiband dynamics, limiting and high-frequency enhancement.

## Plugins

Click a plugin name to open its full-size screenshot.

| Plugin | What it does |
| :--- | :--- |
| [Petal Granulizer](docs/screenshots/petal-granulizer.png) | Granular textures with position and spray, reverse, quantized random pitch and freeze. |
| [Petal Oracle](docs/screenshots/petal-oracle.png) | Generative resonator: six strings move through a selected key and scale, with chaos, glide and freeze. |
| [Petal Retro](docs/screenshots/petal-retro.png) | Lo-fi colour from noise, wobble, distortion, digital degradation, space and magnetic effects. |
| [Petal Crush](docs/screenshots/petal-crush.png) | Level crushing with darkness, release, “Shut Up”, mix and output controls. |
| [Petal Half](docs/screenshots/petal-half.png) | Tempo-synced half-time with selectable speed and grid, smoothing and tone. |
| [Petal EQ](docs/screenshots/petal-eq.png) | Eight-band parametric EQ with a spectrum analyzer and band audition. |
| [Petal Comp](docs/screenshots/petal-comp.png) | Compressor with threshold, ratio, knee, attack, release, makeup gain and a scrolling level display. |
| [Petal Squash](docs/screenshots/petal-squash.png) | Three-band upward and downward compression with per-band controls. |
| [Petal Limit](docs/screenshots/petal-limit.png) | Lookahead brickwall limiter with selectable styles, true-peak mode and LUFS metering. |
| [Petal Air](docs/screenshots/petal-air.png) | Dynamic high-frequency enhancement with mid-air, high-air and trim controls. |
| [Petal Delay](docs/screenshots/petal-delay.png) | Stereo delay with normal, inverted and ping-pong feedback, filtering and tempo sync. |
| [Petal Verb](docs/screenshots/petal-verb.png) | Algorithmic reverb with predelay, size, decay, modulation, damping, width and freeze. |
| [Petal Mod](docs/screenshots/petal-mod.png) | Multi-tap modulation with chorus, ensemble, flanger, vibrato and sky styles. |
| [Petal Double](docs/screenshots/petal-double.png) | Four-voice doubler with individual delay, pitch, gain and pan, plus modulation and feedback. |
| [Petal Shift](docs/screenshots/petal-shift.png) | Pitch shifter with semitones, cents, stereo spread and fast, medium or slow modes. |

More detail: [Granulizer parameters and visualizer](petal-granulizer/README.md) · [Design system](DESIGN.md) *(in Russian)*.

## Installation

The repository contains source code. [Build the plugins](#build) to create the VST3 and CLAP bundles in `dist/`.

**VST3**

1. Close your DAW. Run `install.cmd` from the project folder; it requests administrator rights and waits for FL Studio to close if needed.
2. The installer copies the VST3 bundles from `dist/` into `C:\Program Files\Common Files\VST3`.
3. Open your DAW and rescan plugins.

To preview the installer's actions, run `install.cmd /dry`. For manual installation, copy each complete `Petal Name.vst3` folder into the VST3 directory above.

**CLAP**

Copy the `.clap` files from `dist/` into your DAW's CLAP scan directory, then rescan. `install.cmd` installs VST3 bundles only.

## Build

You need **Windows x64**, a stable [Rust toolchain](https://rustup.rs) and a matching native linker/toolchain, such as MSVC Build Tools or MinGW for the GNU target.

```powershell
git clone https://github.com/b4demus/petalbundle.git
cd petalbundle

# Build and package all 15 plugins into dist/
powershell -ExecutionPolicy Bypass -File build.ps1

# Build just one plugin
powershell -ExecutionPolicy Bypass -File build.ps1 -Only petal-eq
```

Each plugin produces a `.vst3` bundle and a `.clap` file. Build outputs are excluded from Git.

<details>
<summary><strong>Portable GNU toolchain</strong></summary>

`build.ps1` normally uses the system `cargo`. To use a portable Windows GNU setup, set `PETAL_TOOLCHAIN` to a folder containing `cargo/`, `rustup/` and `gcc/w64devkit/`. The script also checks `../.tools/rust-daw` for this layout.

The empty archive in `.cargo/libstub/libgcc_eh.a` is used by this w64devkit setup; the build script sets `LIBRARY_PATH` for it.

</details>

### Development

Run the DSP unit tests:

```powershell
cargo test --workspace --lib
```

Preview an interface without a DAW using a built-in test signal and the dummy audio backend:

```powershell
cargo run --release -p petal-eq --features standalone --bin petal-eq-standalone --target-dir target-standalone -- --backend dummy
```

### Project layout

| Path | Contents |
| :--- | :--- |
| `petal-*/` | Individual plugins: DSP, parameters, nih-plug integration, editors and standalone previews. |
| `shared/` | The `bundle_ui` crate: palette, knobs, cards, buttons and window scaling. |
| `docs/` | The complete plugin collage and individual screenshots. |
| `build.ps1` / `install.cmd` | Windows packaging and VST3 installation. |

To add a plugin, register its crate in the root `Cargo.toml` workspace and the `$plugins` map in `build.ps1`. See the checklist in [DESIGN.md](DESIGN.md).

## License & credits

**[GPL-3.0-or-later](LICENSE)**, as declared in the crate manifests. Built with [nih-plug](https://github.com/robbert-vdh/nih-plug) and its egui integration. The nih-plug VST3 bindings are GPLv3-licensed.

Created by [b4demus](https://github.com/b4demus). Any third-party product names used as sonic references belong to their respective owners; Petal is not affiliated with them.
