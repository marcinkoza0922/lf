# Leaf

A read-only Rust Markdown and UTF-8 text reader for Linux/Wayland and Windows. The executable is named `lf`.

```sh
cargo install --path .
lf README.md
```

**Windows and ARM64 builds have not been runtime-tested.** Release packaging covers
Linux and Windows on x64 and ARM64; see [release instructions](docs/RELEASE.md)
for archives, prerequisites, and build commands.

Use **Open…**, **Ctrl+Shift+O**, or drop a file onto the window. The native file chooser
uses the Windows native dialog on Windows and the desktop portal on Linux; on KDE install and enable `xdg-desktop-portal` and
`xdg-desktop-portal-kde` to get KDE's file dialog. No GTK file chooser is bundled.
`lf --help` prints usage.

Press **Space** or click **Quick select** to find `.md` files recursively under the
**launch directory**, even after opening a document elsewhere. Type a fuzzy query,
use **Tab / Shift+Tab** to select the next/previous match, **Enter** to open, and
**Esc** to close. Files appear with relative paths; hidden directories are
included, and directory symlinks are skipped to avoid loops.

Links to other `.md`, `.markdown`, and `.txt` files open in Leaf (right-click a
link to open it in the default app instead); other local files open in their
default app. Heading links such as `#setup` or `guide.md#setup` jump to the
matching heading (GitHub-style anchors). Images, audio, and video show inline only
when they are alone on their line; elsewhere they appear as links you click to
expand.

**Back / Forward** step through opened documents and heading-link jumps, restoring
each scroll position: **Ctrl+O / Ctrl+I**, **Alt+Left / Alt+Right**, the mouse
back/forward buttons, or the toolbar arrows.

The collapsible **sidebar** sits on the left. Collapsed, it is a slim rail: click
the chevron or a tab icon to expand it, and drag its edge or click the inner
chevron to collapse it again. It has two tabs. **Opened** (**`**) lists documents
opened this session, then recent files from earlier sessions (missing files are
dimmed). **Files** (**Ctrl+E**) is a tree of `.md` and `.txt` files under the
launch directory with the current document revealed.

A tab's key expands the sidebar on that tab and gives it keyboard focus; pressing
it again while focused collapses the sidebar. Clicking in the sidebar also focuses
it, and **Esc** or clicking the document returns focus to the document. While
focused: **j/k** or **↑/↓** move the selection, **Home/End** jump to the ends,
**Enter** opens a file (and returns focus to it) or toggles a folder, **l/→**
expands a folder or opens a file while staying in the sidebar, **h/←** collapses
a folder or moves to its parent, and **Tab / Shift+Tab** switch tabs.

Leaf implements common **Vim navigation bindings**, including **h/j/k/l** cursor
movement, **w/e/b** and **W/E/B** word motions, **gg**, **G**, **0**, **$**,
**f/t/F/T** character finds, **v** selection, and **y** copying. It also supports
**/** search and **n/N** next/previous matches. Leaf remains a read-only viewer.

Scroll the document with **arrow keys**; **Shift+J/K** scroll down/up by a line.
**Page Up/Down** moves by a page with a little overlap. **Home** or **gg** goes
to the top; **End**, **Shift+G**, or **ge** goes to the bottom. Two-key commands
must be typed within one second. These shortcuts pause while a dialog or input
control has focus.

Find text in the current document with **/** or **Ctrl+F**. Matches are highlighted
independently of your mouse selection. **Enter / Shift+Enter** (or Next/Previous)
cycles through matches. **Esc** closes search and clears its highlights without
clearing your text selection.

Press **S** to search text across all `.md` files in the launch directory and its
subdirectories. Type a query and press **Enter** to search; use **Tab / Shift+Tab**
to select a result, then **Enter** to open it at the match. Searches are literal
and case-insensitive. The reader uses `rg` (install it on PATH), falling back to GNU `grep` on Linux.
Results are limited to 1000 matching lines; **Esc** closes the search.

Use **[ / ]** to jump to the previous/next Markdown heading. The destination
heading is highlighted; **Esc** clears the highlight. Fenced code is excluded.
**Ctrl+H** opens a Go to heading popup listing every heading as an indented
outline. Type to fuzzy-filter, use **Up / Down** or **Tab / Shift+Tab** to select
(the heading at your reading position is preselected), and **Enter** to jump;
**Back** returns to where you were. **Esc** or **Ctrl+H** closes it.

Files are checked for changes about every half second. Automatic reload is opt-in;
otherwise a Reload prompt appears. **Ctrl+R** forces a reload.
Failed reloads preserve the last readable version and retry. Files must be UTF-8
and at most 16 MiB. One document is open at a time.

**Appearance** offers installed system fonts, text size, line height, paragraph
gap, and reading width. All changes affect only the view. Choose from Paper,
Solarized Light/Dark, Gruvbox Light/Dark, Dracula, Nord, and Tokyo Night. Automatic
mode switches between your chosen day and night themes using configurable local
hours (07:00–19:00 by default). Preferences survive restarts.

Supports headings, emphasis, lists, read-only task lists, tables, block quotes,
links, syntax-highlighted code, and local images. Embedded HTML is not executed;
math and diagrams are outside this reader's scope. Remote images aren't fetched.

Development:

```sh
cargo nextest run
cargo clippy --message-format=short
cargo fmt --check
```

Architecture and confirmed requirements: [docs/DESIGN.md](docs/DESIGN.md).

Licensed under the [Apache License 2.0](LICENSE). Bundled third-party crates
retain their existing license files.

Reading motions: **w** next word, **e** word end, **b** previous word.
**f** then a character finds it on the current visual line; **t** stops before it.
A caret marks the position and the current word is highlighted.

**W**, **E**, and **B** move by whitespace-separated WORDs, including punctuation.
**F** finds backward on the current visible line; **T** stops just after the target.

**0** moves to the start of the current visible line; **$** moves to its end.
