# Performance profiling

Measured on the development Linux/Wayland machine, release build, November 11,
2026. These are individual local observations, not portable timing guarantees.

## Measurements

Native startup with `README.md` reached application setup in **328 ms** and the
end of the first UI frame in **358 ms** after entering `main`. This excludes
process loader time and does not measure compositor presentation. No native
before-change startup measurement was taken. Startup previously always scanned
system fonts; isolated scans ranged from **28 ms warm to 1.4 s initially**, and
some subsequent separate processes took over 2 s. Default built-in-font startup
now skips this scan. Persisted custom fonts still require discovery; first opening
preferences pays the deferred discovery cost.

The headless workload repeats this UTF-8 Markdown block to just below the target
size (the 16 MiB fixture has 217,885 headings):

```markdown
## Heading

A paragraph with **bold** and Unicode café text for profiling.

```

| Headless CPU work | Before | After |
| --- | ---: | ---: |
| 1 MiB first frame | 249 ms | 110 ms |
| 1 MiB subsequent frames | 228–241 ms | 77–98 ms |
| 1 MiB initial text indexing | 150 ms | 15 ms |
| 1 MiB cached text indexing | rebuilt each frame | 1.7 ms |
| 16 MiB first frame | 66.1 s | 1.8–2.0 s |
| 16 MiB initial text indexing | 64.4 s | 243–250 ms |
| 16 MiB subsequent frames | not measured | 1.76–1.85 s |
| 16 MiB cached text indexing | rebuilt each frame | 27–36 ms |

File reads were about 1–2 ms at 1 MiB and 21–24 ms at 16 MiB. Early metadata
rejection now avoids reading an oversized file; bounded reads still enforce the
limit if the file grows after the metadata check.

Other shapes: a 1.1 MiB single paragraph took 212 ms for its first frame and
3 ms on the second; a roughly 1 MiB fenced Rust code block took 88 ms initially
and 28 ms on the second frame. These use the same helper, with default renderer
styles. The helper is a component benchmark, not an exact reproduction of the
application's persisted typography or native GPU work.

## Findings and changes

The largest measured hotspot was quadratic heading indexing: each heading glyph
searched all previous headings. Accumulating into the current heading makes that
work linear. The application now reuses its index when galley identities and
relative positions match, translating glyph geometry on scroll and rebuilding
after reflow or content changes.

Cursor geometry previously searched all glyphs for each character visited. Vertical
motions, viewport motions, mouse selection synchronization, and long visual
selections could consequently perform quadratic work. Lookup now uses binary
search, and hidden idle cursors avoid building character/offset arrays. Search
highlight ranges also use ordered byte offsets instead of scanning every glyph
for every match. Existing cursor/search tests exercise these paths; the reported
benchmark cursor times cover the hidden idle cursor, not interactive motions.

The remaining bottleneck is full-document Markdown parsing/widget layout on every
frame. The 16 MiB fixture still takes roughly 1.6–1.8 s to render per frame, and
peak RSS for two frames was **2,443,048 KiB (2.33 GiB)**. Files near the limit are
therefore still unsuitable for smooth scrolling. Viewport virtualization with a
complete offscreen search/cursor index would require a further renderer change;
the existing renderer's scrollable mode does not preserve the complete layout
reports that those features need. The 16 MiB size limit remains in force.

## Reproduction

```sh
cargo build --release
python3 - <<'PY'
from pathlib import Path
block = '## Heading\n\nA paragraph with **bold** and Unicode café text for profiling.\n\n'
for size in (65536, 1048576, 16777216):
    Path(f'/tmp/md-profile-{size}.md').write_text(
        block * (size // len(block.encode())), encoding='utf-8')
PY
target/release/md-reader --profile /tmp/md-profile-1048576.md 3
/usr/bin/time -f 'peak_rss_kib=%M elapsed_s=%e' \
    target/release/md-reader --profile /tmp/md-profile-16777216.md 2
MD_READER_PROFILE_STARTUP=1 target/release/md-reader README.md
```

`tools/profile.rs` is compiled through a path module in `main.rs`. Its font scan
is intentionally isolated and always runs, even though default native startup
now skips it. Frame timings exclude that scan and file loading. The helper does
not open a window, tessellate/upload to a GPU, or time screen presentation.

Validation: `cargo nextest run` (41 passing tests),
`cargo clippy --message-format=short`, and `cargo fmt --check`.
