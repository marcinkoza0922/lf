# Design

## Requirements and interview decisions

A read-only Markdown desktop reader in Rust for Linux on Wayland. One document
at a time; open through the desktop's native portal file chooser, drag and drop,
or a command-line path. Under KDE this is the KDE file dialog (the same desktop
integration used by Dolphin), rather than embedding Dolphin itself.
Basic Markdown only; no math, Mermaid, or executable HTML. Installed system
fonts, adjustable text size, extra color themes, and preferences persisted
between launches were confirmed. Automatic day/night themes follow local time.
Automatic reload is available as an opt-in reading preference; it defaults to off.

## Architecture

Use eframe/egui with the native Wayland and OpenGL backend. egui_commonmark
renders Markdown (headings, lists, tables, links, code, local images), with
syntect code highlighting. There is no source editor. egui_extras loads local
images. No asset creation pipeline is needed.

Preferences are serialized by eframe into its application storage. Fonts are
listed using fontdb and loaded on selection, with built-in fonts as fallbacks.
Themes: Paper, Solarized Light, Gruvbox Light, Dracula, Gruvbox Dark, Nord,
Solarized Dark, and Tokyo Night. Automatic mode uses configurable local-hour
boundaries (default 07:00 inclusive to 19:00 exclusive), with independent day
and night selections; manual mode remains available.

The file chooser runs in a worker thread and returns through a channel, keeping
the UI responsive. A single document owns its canonical path, UTF-8 content,
and error state. Every 500 ms, reread the path and compare contents: this handles
atomic editor renames, coarse timestamps, deletion/recreation, and same-length
edits. With automatic reload off (the default, including older saved preferences),
keep the displayed text and show a nonmodal change prompt with a Reload button.
The prompt remains while disk contents differ and clears when they match again
or a reload succeeds. `r` outside text entry and modals, Ctrl+R, and the toolbar
Reload button read the latest contents. The reading preferences include a persisted
checkbox to enable automatic reload. Keep the last successful render on read
errors and retry automatically.
A 16 MiB document limit bounds work; file loading is synchronous within this
limit. Scroll identity stays stable across reloads and resets on opening a new
file. Local image loader caches are cleared on reload. File polling also
refreshes local time, so theme changes take effect while idle.

## Validation and operation

`cargo nextest run` tests schedule boundaries and rollover, document reload,
invalid UTF-8, file size limit, and recovery after deletion/atomic replacement.
`cargo clippy --message-format=short` checks the application; `cargo fmt --check`
checks formatting. `cargo run -- path/to/file.md` launches the reader. The KDE
portal service and its KDE backend must be installed for the native chooser.
No helper tools are currently required; future helpers belong in `tools/`.

## Visible spacing and renderer extension

The user additionally requested adjustable visible spacing without changing the
source. Preferences include line height (1.0–2.4 times text size), paragraph gap
(0–40 logical points), and reading width (360–1400 points, capped by the window).
These values are persisted along with the other preferences.

`crates/egui_commonmark` is a local copy of upstream 0.25.0 (MIT/Apache licenses
retained). The small extension adds read-only line-height and paragraph-separator
options to the existing renderer; this avoids modifying Markdown to simulate
spacing. Headings and syntax-highlighted code retain their natural line heights.
Image destinations are resolved against the document directory using URL joining,
including absolute paths, parent paths, and paths containing spaces. Network image
fetching is not enabled. Headless egui rendering tests verify that spacing changes
rendered geometry and that representative Markdown renders successfully.

## Verified implementation

Validation completed with twenty-one passing nextest tests, a clean clippy run, and
`cargo fmt --check`. A native Wayland launch with README.md stayed alive for an
eight-second smoke run with no logged runtime errors. The smoke run was ended
by timeout; file chooser interaction, drag-and-drop, and visual polish have not
been verified interactively. Source modifications to the local renderer are
listed in `crates/egui_commonmark/PATCHES.md`.

## Quick select

Press F (outside text/numeric entry) or use Quick select in the toolbar to list
all `.md` files recursively under the launch directory, captured once at startup.
Opening documents in other directories never changes this root. Paths are shown
relative to this root and sorted. Hidden directories are included; directory
symlinks are not followed, preventing traversal cycles. Symlinks to regular
Markdown files are included. The scan runs on a cancellable background thread,
reports skipped unreadable entries, and is refreshed each time the chooser opens.

The chooser is modal with an initially focused fuzzy search field. Matching is
case-insensitive ordered-subsequence matching over relative paths, ranked to favor
contiguous matches and filename/word boundaries; ties follow path order. Empty
queries show all files in path order. Typing recomputes matches and resets selection.
Tab selects the next match; Shift+Tab selects the previous one, stopping at list
boundaries and keeping the selection visible. Search retains focus during
navigation, so all letters (including H/J/K/L) are ordinary query text. Enter
opens the selected match and Escape closes the chooser. Clicking a match opens it.

Quick-select tests cover recursive scans, ranking and Unicode subsequences,
Tab/Shift+Tab selection, typing after navigation, opening a filtered match,
empty matches, and Escape dismissal. Clippy and formatting checks pass.

## Document keyboard navigation

Arrow Up/Down and K/J move vertically by one reading line; Left/Right and H/L
move horizontally when wide content overflows. The document scroll area supports
both axes while prose continues wrapping to the reading width. Page Up/Down move
90% of the visible document height, retaining context. Home and `gg` go to the
top; End, Shift+G, and `ge` go to the bottom. Two-key sequences have a one-second
window, ignore held-G repeats, and reset on unrelated keys, opening a new document,
or entering a modal/text control. Keyboard deltas apply to the document scroll
area only, using its current content and viewport bounds, and clamp at both ends.
Quick select, the file chooser, and focused appearance controls retain their own
keyboard behavior. Tests cover sequence timing, repeats, suppression, and actual
headless scroll offsets, including short documents and overflow on both axes.

## Search and heading navigation

`/` and Ctrl+F open case-insensitive literal search over rendered document text.
The slash shortcut accepts character events as well as Slash key events, including
keyboard layouts where typing `/` requires Shift, and consumes the opening
character so it does not enter the query.
All occurrences receive translucent amber highlights, with the active occurrence
in orange. Enter/Shift+Enter navigate matches and release search input focus. Outside text
entry, `n`/Shift+N navigate next/previous matches; next/previous controls also
navigate matches. Escape
closes search and removes search/heading overlays without modifying user selection.
`[` and `]` jump to previous/next parsed Markdown headings relative to the current
viewport, with a separate heading highlight. Headings include setext headings;
fenced-code hash signs are not headings. Navigation stops at document boundaries.

The local Markdown renderer and backend expose a per-render text report with
actual glyph layout, source spans, and parsed heading identities, including
links, tables, and highlighted code. This allows matching across formatting spans,
scrolling to offscreen matches, and overlays independent of egui's selection state.
Reports are rebuilt after content or layout changes, keeping autoreload correct.

`S` opens recursive literal content search in all `.md` files under the captured
launch directory. Execute `rg` directly with fixed-string, case-insensitive,
hidden/no-ignore, Markdown-glob, line-number and NUL-filename flags. If `rg` is
missing, execute recursive `grep` with equivalent flags. No shell interpolation
is used. Queries run on background threads; processes are killed on cancellation
or replacement, results are capped at 1000, and partial traversal errors are
reported. Enter submits a changed query, then opens the selected result; Tab and
Shift+Tab navigate results. Opening a result highlights the query and scrolls to
the corresponding source line. Paths containing colons/newlines are parsed using
NUL delimiters rather than splitting filenames on punctuation.

Validation for search includes actual drag selection and clipboard copying before
and after highlighting/Escape, offscreen match and heading scrolling, Unicode and
formatting-spanning matches, search focus release and match shortcuts, rg/grep parity on unusual
filenames, literal query argument handling, and result limits. Twenty-one tests
pass, with clean clippy and formatting checks. Recursive search is over source
text; markup-only matches scroll to the closest rendered text when there is no
visible occurrence to highlight. Renderer reports are collected only while
search or heading overlays are active.
