#!/usr/bin/env python3
"""Package an already built Leaf binary with documentation and licenses."""
import argparse
import hashlib
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[1]
TARGETS = {
    "x86_64-unknown-linux-gnu": "linux-x64",
    "aarch64-unknown-linux-gnu": "linux-arm64",
    "x86_64-pc-windows-msvc": "windows-x64",
    "aarch64-pc-windows-msvc": "windows-arm64",
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True, choices=TARGETS)
    parser.add_argument("--binary", type=Path, help="Override target/<triple>/release/lf[.exe]")
    parser.add_argument("--output", type=Path, default=ROOT / "target/packages")
    args = parser.parse_args()
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["package"]["version"]
    windows = "windows" in args.target
    executable = "lf.exe" if windows else "lf"
    binary = args.binary or ROOT / "target" / args.target / "release" / executable
    if not binary.is_file():
        parser.error(f"Build the release binary first: {binary}")
    args.output.mkdir(parents=True, exist_ok=True)
    name = f"lf-{version}-{TARGETS[args.target]}"
    with tempfile.TemporaryDirectory() as temporary:
        package = Path(temporary) / name
        package.mkdir()
        shutil.copy2(binary, package / executable)
        for filename in ["README.md", "LICENSE", "docs/RELEASE.md"]:
            shutil.copy2(ROOT / filename, package / Path(filename).name)
        for source in sorted((ROOT / "crates").glob("*/LICENSE*")):
            destination = package / "licenses" / source.parent.name / source.name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)
        revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
        dirty = bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT))
        (package / "BUILD.txt").write_text(
            f"Leaf {version}\nTarget: {args.target}\nSource: {revision}"
            f"{' (modified working tree)' if dirty else ''}\n"
            "Windows and ARM64 builds have not been runtime-tested.\n", encoding="utf-8")
        archive = args.output / (name + (".zip" if windows else ".tar.gz"))
        if windows:
            with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as output:
                for file in sorted(package.rglob("*")):
                    if file.is_file():
                        output.write(file, file.relative_to(package.parent))
        else:
            with tarfile.open(archive, "w:gz") as output:
                output.add(package, arcname=name)
    with archive.open("rb") as source:
        digest = hashlib.file_digest(source, "sha256").hexdigest()
    archive.with_name(archive.name + ".sha256").write_text(f"{digest}  {archive.name}\n")
    print(archive)


if __name__ == "__main__":
    main()
