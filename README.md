# lf

A read-only Rust Markdown reader for Linux/Wayland.

```sh
cargo install --path .
lf README.md
```

Use **Open…**, **Ctrl+O**, or drop a file onto the window. The native file chooser
uses the desktop portal; on KDE install and enable `xdg-desktop-portal` and
`xdg-desktop-portal-kde` to get KDE's file dialog. No GTK file chooser is bundled.
`lf --help` prints usage.

Press **Space** or click **Quick select** to find `.md` files recursively under the
**launch directory**, even after opening a document elsewhere. Type a fuzzy query,
use **Tab / Shift+Tab** to select the next/previous match, **Enter** to open, and
**Esc** to close. Files appear with relative paths; hidden directories are
included, and directory symlinks are skipped to avoid loops.

Scroll the document with **arrow keys** or **H/J/K/L** (left/down/up/right).
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
and case-insensitive. The reader uses `rg`, falling back to `grep` if unavailable.
Results are limited to 1000 matching lines; **Esc** closes the search.

Use **[ / ]** to jump to the previous/next Markdown heading. The destination
heading is highlighted; **Esc** clears the highlight. Fenced code is excluded.

Files reload automatically within about half a second. **Ctrl+R** forces a reload.
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
