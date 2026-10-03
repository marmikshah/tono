#!/usr/bin/env python3
"""Install a built tono wheel in a fresh environment and run the Python suites."""

import argparse
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import venv


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path, help="directory containing wheels compatible with this Python/platform")
    args = parser.parse_args()
    directory = args.directory.resolve()
    if not list(directory.glob("tono-*.whl")):
        parser.error(f"no tono wheels in {directory}")
    root = Path(__file__).resolve().parents[1]
    with tempfile.TemporaryDirectory(prefix="tono-wheel-") as folder:
        venv.EnvBuilder(with_pip=True).create(folder)
        python = Path(folder) / ("Scripts/python.exe" if os.name == "nt" else "bin/python")
        commands = [
            ["-m", "pip", "install", "numpy"],
            ["-m", "pip", "install", "--no-index", "--find-links", str(directory), "--no-deps", "tono"],
            [str(root / "crates/tono-py/tests/smoke.py")],
            [str(root / "crates/tono-py/tests/test_typed.py")],
        ]
        for command in commands:
            subprocess.run([str(python), *command], cwd=root, check=True)


if __name__ == "__main__":
    try:
        main()
    except (OSError, subprocess.CalledProcessError) as error:
        sys.exit(f"wheel test: {error}")
