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

Press Space (outside text/numeric entry) or use Quick select in the toolbar to list
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

Arrow Up/Down scroll vertically by one reading line; Left/Right
scroll horizontally when wide content overflows. H/J/K/L are consumed by the
reading cursor before scroll navigation. The document scroll area supports
both axes while prose continues wrapping to the reading width. The reading
column is centered using the scroll viewport width, with 12-point minimum side
margins, independent of overflowing content. Prose explicitly uses word wrapping
(including long words) within that column. Page Up/Down move
90% of the visible document height, retaining context. Home and `gg` go to the
top; End, Shift+G, and `ge` go to the bottom. Two-key sequences have a one-second
window, ignore held-G repeats, and reset on unrelated keys, opening a new document,
or entering a modal/text control. Keyboard deltas apply to the document scroll
area only, using its current content and viewport bounds, and clamp at both ends.
Quick select, the file chooser, and focused appearance controls retain their own
keyboard behavior. Tests cover sequence timing, repeats, suppression, and actual
headless scroll offsets, including short documents and overflow on both axes.

## Search and heading navigation

`/` and Ctrl+F open a centered, nonmodal search window over rendered document text.
Search defaults to case-insensitive literal matching. Match case, Fuzzy, and Regex
checkboxes control matching; fuzzy and regex are mutually exclusive. While the
search window is open, Ctrl+I toggles Match case, Ctrl+U toggles Fuzzy, and Ctrl+Shift+R
toggles Regex, including during text entry. These keys are consumed before global
shortcuts; Ctrl+X retains its cut behavior and Ctrl+R reloads normally. Copy and Ctrl+F retain
their existing behavior. Fuzzy uses
ordered subsequences within a line and highlights the complete matched span. Regex
uses the Rust regex engine, shows invalid-pattern errors inline, and ignores
zero-width matches. Changing options resets the active match just like changing
the query.
The slash shortcut accepts character events as well as Slash key events, including
keyboard layouts where typing `/` requires Shift, and consumes the opening
character so it does not enter the query.
All occurrences receive translucent amber highlights, with the active occurrence
in orange. Enter/Shift+Enter close the search window and release input focus, preserving
the query, highlights, and active match. Opening search focuses the input and
selects the entire existing query, so typing replaces it (including Unicode text). Outside text
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
visible occurrence to highlight. Renderer reports are collected for the visible reading cursor as well as search
and heading overlays.


## Reading cursor and Vim word motions

The document uses a translucent highlight over its current word (or punctuation
run), with no caret. The highlight is initially hidden and cursor memory resets
on opening a new document.
`w` moves to the next word start, `e` to the next word end, and `b` to the previous
word start. `W`, `E`, and `B` use whitespace-separated WORDs, including
punctuation, and highlight the entire current WORD. Unicode letters/numbers and underscores form words; punctuation forms
separate runs. Boundaries clamp to the document. `ge` retains its existing
bottom-of-document binding. `0` moves to the first character of the current
visual line (including leading whitespace), and `$` moves to its last character.
These motions follow soft wrapping; character events support shifted keyboard
layouts, and pending `f`/`t`/`F`/`T` targets take precedence.

`f` followed by a character moves onto its next occurrence on the current visual
line; `t` stops just before it. `F` finds the previous occurrence on the same
visual line and `T` stops just after that occurrence. Both directions exclude
the current character. Failed finds leave the cursor in place. Escape
cancels a pending character target. Target characters are consumed before app
shortcuts, and motions are suppressed in text controls, appearance controls, and
modals. Cursor movement scrolls the target into view and preserves clipboard
selection. Layout reports provide character rectangles across Markdown formatting
and reflow. After reload the character position is clamped to the current text.
The Space shortcut opens the fuzzy file finder, freeing `f` for character finds.

Tests cover lowercase and shifted keys, Unicode whitespace and punctuation
boundaries, pending character input,
shortcut suppression, and character finds using actual rendered line geometry.


`h`/`l` move the reading cursor one Unicode character backward/forward, clamped
at document boundaries. `j`/`k` move to the nearest character by horizontal
position on the next/previous rendered line, including soft wraps. Arrow keys
retain scrolling behavior. `v` toggles continuous character selection anchored
at the cursor; all cursor motions extend it, including both endpoints. Escape
cancels it. `y` or Ctrl+C copies the selected rendered text to the system clipboard
and exits selection mode. Without a keyboard selection, Ctrl+C retains native
mouse-selection copying. Selection and cursor positions clamp after reload.

Character motions (`h`/`j`/`k`/`l`) highlight only the current character outside
visual selection mode; word motions restore word highlighting. The first
character motion after opening a document activates the cursor at its first
non-whitespace character, without advancing. Merely rendering the document does not activate it.
Subsequent character motions move normally; `v` continues extending the full
anchored selection.

Shift+J and Shift+K scroll down/up by one reading line without moving or
activating the reading cursor or changing an anchored selection. Pending
character-find targets retain precedence over these shortcuts.

Escape and a primary click inside the document hide the keyboard highlight and
cancel visual selection while preserving the last keyboard position. Escape
also retains search dismissal behavior. Resuming keyboard motions advances from
that remembered position. A new mouse text selection replaces it with the
character nearest the mouse selection's active endpoint, including backward
drags. Starting keyboard navigation clears the native mouse selection so only
the keyboard highlight remains. Shift+J/K scrolling does not reveal a hidden
highlight.
