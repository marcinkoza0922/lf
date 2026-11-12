# lf

Write a markdown renderer in Rust. Users should be able to change font type + scale, as well as color themes (use popular ones like dracula, gruvbox, etc.).

No editing, this is just a reader.

Also support UTF-8 `.txt` files, displaying their text literally without Markdown rendering.

Render linked images inline. Audio and video attachments play inside the reader
with explicit playback controls. Other local file links render as cards with an
icon, label, filename, and an action to open in the system's default application.
Web page links continue opening in the browser.
