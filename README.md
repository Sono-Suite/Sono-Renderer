# Sono Renderer

Please scroll down to find the AI disclosure section.

This is a basic Sonolus renderer, with compatibility suited for Next Sekai, Next Rush, and Horizon charts.

This project has a basic GUI to help you navigate it easier.

SCP files and engine files are either included or in Sono-Server.

## What it does

- Renders a MP4 file.
- Supports BGM, engine sound effects, particles, optional video backgrounds,
  gameplay UI profiles, and watermark settings.
- Supports WGPU natively. CPU rendering is left as a failsafe.
- Two types of renderers are supported: Sono-GCC and Sono-VM. Sono-GCC is the native compiled renderer.
- Provides render jobs, history, profiles, configuration files, diagnostics,
  and content-based render manifests.
- Preview is included, which renders a specific frame.

## Requirements and platform status

- Windows x86-64 is natively supported.
- A web implementation is currently in the works...

## Build and launch

From a source checkout on Windows:

```powershell
cargo build --release --offline
cargo run --release -- gui
```

See [Installation](docs/INSTALLATION.md) for release availability, portable installs, FFmpeg, data storage, and updates.

## Quick Start

1. Open **Quick Render**.
2. Select an engine ZIP, a Sonolus LevelData chart, and its resource package.
   The renderer accepts standalone JSON or gzip-compressed JSON LevelData and
   legacy level ZIP packages; resources are supplied as SCP/ZIP packages.
3. Choose music when **Include music** is on, then set the output MP4, range,
   resolution, and frame rate.
4. Select **Check inputs** and review any compatibility findings.
5. Select **Queue render**. Follow progress in **Jobs & History**, then open
   the completed video or its output folder.

Projects and saved render profiles are optional. Dropping files or a folder
can discover input roles; ambiguous matches require a choice. See
[Getting Started](docs/GETTING_STARTED.md) for the full workflow.

## CLI rendering

This project supports command line inputs:

```text
cargo run --release -- render-video <engine.zip> <resources.scp> <level.json.gz> <music.ogg> <output.mp4> --whole-chart
```

If you need more fine tuning: Use `cargo run --release -- --help` and the command-specific `--help` for the
current options.

See [shared configuration](docs/USER_CONFIGURATION.md) and
[workspaces and commands](docs/WORKSPACE_COMMANDS.md).

## Rendering and compatibility scope

See these files:
- [compatibility analyzer](docs/COMPATIBILITY_ANALYZER.md)
- [current limitations](docs/KNOWN_LIMITATIONS.md)
- [coverage matrix](docs/development/sono-gcc/SONO_GCC_COVERAGE.md)

TL:DR, This renderer uses the Sonolus Watch data for chart rendering, does not implement play or preview modes. Sonolus v1.1.5 features are technically implemented, but may not work correctly.

## Documentation

The [documentation index](docs/README.md) organizes user guides, technical
references, development and testing material, compatibility notes, and
historical reports. The Chart Studio and Presentation Studio implementation
guides are marked as retained references because those routes are hidden from
the default desktop navigation.

## Contributing, attribution, and license

For development context, start with the [architecture guide](docs/ARCHITECTURE.md)
and [workspace guide](docs/WORKSPACE_COMMANDS.md). The code is licensed under
the [MIT License](LICENSE). Repository artwork and media may have separate
distribution terms; see [installation notes](docs/INSTALLATION.md).

legal jargan lol:

I do not claim to have written any of the code in this project, as I do not personally know the programming language Rust. This project is AI assisted "to the absolute extreme", and if you don't like that, it's fine. However, please make the distinction that I am not going to turn my YouTube content flow and actual creative works into AI slop machine. AI content slop is not something that I endorse in any way, shape or form. Please understand my intention with this project is not to claim that I wrote the code in this project, but to provide a tool that helps automate the chart production process much much quicker.

Acknowledgements:

Zihad - Sonolus Renderer Code provided as reference

LittleYang0531 - More rendering code provided as reference

qwewqa / Hyeon - Next Sekai Preview implementation for development reference

Burrito (Sonolus) - literally the entire reason this exists, W

Being mentioned here does not necessarily mean that they have endorsed this project.
Credit is given to acknowledge the people who made related works that contribute to its testing.

All developers whose code was provided to AI as a development reference were made explicitly aware and gave permission for my use case of their code. Again, this permission does not imply endorsement of Sono-Renderer.

This program was built and tested against Sonolus v1.1.4. There is limited support for Sonolus v1.1.5, as it's not yet out.