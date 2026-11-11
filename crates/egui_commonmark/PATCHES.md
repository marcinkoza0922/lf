# Local changes from egui_commonmark 0.25.0

Source: https://github.com/lampsitter/egui_commonmark (crates.io release 0.25.0).
The upstream MIT and Apache-2.0 licenses are retained alongside the source.

- Add optional `line_height` and `paragraph_spacing` viewer builders. Forward
  their values to renderer state; apply line height to body RichText and gap
  height to paragraph separator labels. Existing default behavior is preserved
  when these options are absent. Headings and highlighted code stay unchanged.
- Resolve image URLs against the configured implicit base with `url::Url::join`
  so local absolute paths and relative paths work with encoded directory names.

- Add `show_with_report`, returning actual glyph layouts, source spans, and
  heading identities for read-only search/heading overlays. The local backend
  records normal labels, links, breaks, and code, even outside the viewport;
  normal labels retain egui's wrapping and user-selection behavior.

The application owns persistence and UI controls. Geometry coverage lives in
`src/main.rs` of the application. This copy is a path dependency, not an
independent fork intended for publication.

- Add `show_plain_text` for selectable literal text with source-aware reports.

- Add `render_attachment_fn` for application-owned link/image cards and players,
  and re-export `attachment_label` to retain source-aware text layout reports.
