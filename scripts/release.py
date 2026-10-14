#!/usr/bin/env python3
"""Check release metadata and prepare Linux release artifacts."""

import argparse
import hashlib
from pathlib import Path
import shutil
import tomllib


ROOT = Path(__file__).resolve().parents[1]


def check_version(tag):
    workspace = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]
    version = workspace["package"]["version"]
    if workspace["dependencies"]["tono-core"]["version"] != version:
        raise ValueError("tono-core dependency must match the workspace version")
    for member in workspace["members"]:
        package = tomllib.loads((ROOT / member / "Cargo.toml").read_text())["package"]
        if package["version"] != {"workspace": True}:
            raise ValueError(f"{member} must inherit the workspace version")
    if tag is not None and tag != f"v{version}":
        raise ValueError(f"tag {tag} does not match workspace version {version}")
    print(f"Release metadata matches {version}")


def checksums(directory):
    artifacts = sorted(p for p in directory.iterdir() if p.is_file() and p.suffix != ".sha256")
    if not artifacts:
        raise ValueError(f"no artifacts in {directory}")
    for artifact in artifacts:
        with artifact.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        sidecar = artifact.with_name(artifact.name + ".sha256")
        sidecar.write_text(f"{digest}  {artifact.name}\n", encoding="ascii", newline="\n")
        print(sidecar)


ARTIFACTS = {
    "x86_64-unknown-linux-gnu": "tono-linux-x86_64",
    "aarch64-apple-darwin": "tono-macos-aarch64",
    "x86_64-apple-darwin": "tono-macos-x86_64",
    "x86_64-pc-windows-msvc": "tono-windows-x86_64.exe",
}


def package_cli(target_dir, out, target):
    try:
        name = ARTIFACTS[target]
    except KeyError:
        raise ValueError(f"unknown target {target}; known: {', '.join(ARTIFACTS)}") from None
    binary = "tono.exe" if name.endswith(".exe") else "tono"
    source = target_dir / target / "release" / binary
    out.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, out / name)
    checksums(out)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    version = commands.add_parser("check-version", help="validate Cargo versions and an optional tag")
    version.add_argument("tag", nargs="?", help="release tag, such as v1.11.0")
    package = commands.add_parser("package-cli", help="copy a built CLI binary into dist with its SHA-256 sidecar")
    package.add_argument("--target", default="x86_64-unknown-linux-gnu", help="Rust target triple the binary was built for")
    package.add_argument("--target-dir", type=Path, default=ROOT / "target")
    package.add_argument("--out", type=Path, default=ROOT / "dist")
    checksum = commands.add_parser("checksums", help="write SHA-256 sidecars for files in a directory")
    checksum.add_argument("directory", type=Path)
    args = parser.parse_args()
    try:
        if args.command == "check-version":
            check_version(args.tag)
        elif args.command == "package-cli":
            package_cli(args.target_dir, args.out, args.target)
        else:
            checksums(args.directory)
    except (OSError, ValueError, KeyError) as error:
        parser.exit(1, f"release: {error}\n")


if __name__ == "__main__":
    main()
