# AGENTS.md
- Language: Rust.
- Specs: `PROPOSAL.md` (what), `docs/DESIGN.md` (how; keep updated).
- Tests: `cargo nextest run` (quiet profile in `.config/nextest.toml`). Never
  run verbose test output unless debugging a specific failure.
- Lint/build: `cargo clippy --message-format=short`; `.cargo/config.toml` is
  set to quiet. Truncate long output.
- If requirements are unclear, ask before assuming.
- You may create any helper tools/functionality needed to build, test, or
  debug. Put them in `tools/` and document them in DESIGN.md.

## Asset tools
- Blender: available via MCP. 3D modeling, scenes, rendering.
- ArtCraft (pure Rust, installed locally). Use only when the project needs
  that asset type:
  - Photocraft: raster editing, layers/masks, PSD
  - VectorCraft: vector illustration
  - SoundCraft: DAW, multitrack/MIDI/mixing, CLI-scriptable
  - MaterialCraft: procedural node-graph materials, exports maps
  - TextureCraft: 3D PBR texture painting, mesh map baking
  - EffectCraft: motion graphics/VFX compositing, render queue
  - FilmCraft: video editing, color, sound
- Invocation notes (fill in as discovered):

## Content search tools
- Discover: `rg --help`, `grep --help`.
- Search (working directory = launch directory): `rg --no-config --null --line-number --with-filename --no-heading --color never --fixed-strings --ignore-case --hidden --no-ignore --glob "*.[mM][dD]" -- QUERY .`
- Fallback when rg is missing: `grep -r -n -H -Z -i -F -I --include="*.[mM][dD]" -- QUERY .`
- Spawn with process arguments, never interpolate queries into shell commands.
