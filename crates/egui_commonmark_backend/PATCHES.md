# Local changes from egui_commonmark_backend 0.25.0

Source: https://github.com/lampsitter/egui_commonmark (crates.io 0.25.0).
Upstream MIT and Apache-2.0 licenses are retained.

`report.rs` optionally collects glyph layout with source/heading metadata for
one viewer render. Normal text uses Label::layout_in_ui and the public
LabelSelectionState API to preserve wrapping, painting, and user selection.
Links report their laid-out galleys; code blocks report TextEdit's immutable
output galley. Explicit line breaks are included in the rendered text index.
Recording is inactive outside `show_with_report`; it never mutates selection.
