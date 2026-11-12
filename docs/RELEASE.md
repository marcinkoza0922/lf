# Release packages

Portable binary archives are prepared for Linux x64, Linux ARM64, Windows x64,
and Windows ARM64. **Windows and ARM64 builds have not been runtime-tested.**
Successful compilation does not imply verified GUI, file chooser, or media behavior.

Linux archives use `.tar.gz`; Windows archives use `.zip`. Extract the archive and
run `lf FILE` or `lf.exe FILE`. Documentation, licenses, build identity, and an
external SHA-256 checksum accompany each package. These are unsigned archives.

Linux requires Wayland, OpenGL, and a desktop portal/file chooser backend.
CI Linux builds use Ubuntu 22.04 (glibc 2.35 baseline); a locally built package
inherits the build host's glibc requirements. Windows uses its native file chooser
and file associations. Windows packages target MSVC; install Microsoft's matching
Visual C++ runtime if the system reports a missing runtime DLL.

Install `rg` (ripgrep) on PATH for recursive content search. Linux also supports
GNU grep as a fallback. Neither search tool is bundled. Reading, current-document
search, and quick select work without these tools.

Inline audio/video is optional. Linux needs `libmpv.so.2` (`mpv-libs` on Fedora,
`libmpv2` on Debian/Ubuntu). Windows needs a matching-architecture `mpv-2.dll`
and its dependencies beside `lf.exe`. Media libraries are not bundled.

## Building packages

Run the **Release packages** GitHub Actions workflow manually or push a `v*` tag.
All four native builds upload archives and checksums as workflow artifacts;
the workflow does not publish a GitHub release. Download artifacts from the run.
The workflow uses Rust stable and the committed Cargo.lock.

For a local package (Python 3.11+ required):

```sh
cargo build --locked --release --target x86_64-unknown-linux-gnu
python3 tools/package-release.py --target x86_64-unknown-linux-gnu
```

Replace the target with `aarch64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`,
or `aarch64-pc-windows-msvc` on the corresponding native build machine.
Output goes to `target/packages/`. Use `--binary PATH` to package a binary built
without an explicit Cargo target, and `--output DIR` to change the destination.
The helper packages an existing binary; it never cross-compiles or downloads tools.
