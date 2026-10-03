#!/usr/bin/env python3
"""Check release metadata and prepare artifacts on Linux, macOS and Windows."""

import argparse
import hashlib
import json
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
    desktop = json.loads((ROOT / "crates/tono-desktop/tauri.conf.json").read_text())
    if desktop.get("version", version) != version:
        raise ValueError("desktop version must match the workspace version")
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


def package_cli(target, name, target_dir, out):
    suffix = ".exe" if "windows" in target else ""
    source = target_dir / target / "release" / f"tono{suffix}"
    out.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, out / f"tono-{name}{suffix}")
    checksums(out)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    version = commands.add_parser("check-version", help="validate Cargo/desktop versions and an optional tag")
    version.add_argument("tag", nargs="?", help="release tag, such as v1.11.0")
    package = commands.add_parser("package-cli", help="copy a target's CLI binary and write its SHA-256 sidecar")
    package.add_argument("target", help="Rust target triple")
    package.add_argument("name", help="artifact platform name, such as linux-x86_64")
    package.add_argument("--target-dir", type=Path, default=ROOT / "target")
    package.add_argument("--out", type=Path, default=ROOT / "dist")
    checksum = commands.add_parser("checksums", help="write SHA-256 sidecars for files in a directory")
    checksum.add_argument("directory", type=Path)
    args = parser.parse_args()
    try:
        if args.command == "check-version":
            check_version(args.tag)
        elif args.command == "package-cli":
            package_cli(args.target, args.name, args.target_dir, args.out)
        else:
            checksums(args.directory)
    except (OSError, ValueError, KeyError) as error:
        parser.exit(1, f"release: {error}\n")


if __name__ == "__main__":
    main()
