#!/usr/bin/env python3
import argparse
import hashlib
import json
import os
import shutil
import subprocess
import tempfile
import time
from pathlib import Path


def desktop_string(value):
    return value.replace("\\", "\\\\")


def desktop_program(path):
    value = str(path)
    if any(character in value for character in "=%") or any(
        ord(character) < 32 for character in value
    ):
        raise ValueError(
            "Installation paths cannot contain '%', '=' or control characters"
        )
    for character in ["\\", '"', "`", "$"]:
        value = value.replace(character, "\\" + character)
    return desktop_string('"' + value.replace("%", "%%") + '"')


def install(source, prefix):
    manifest = json.loads((source / "manifest.json").read_text())
    licenses = (
        "share/doc/glimpse/LICENSE",
        "share/doc/glimpse/LICENSE-lucide",
        "share/doc/glimpse/THIRD-PARTY.txt",
    )
    required = {
        "bin/glimpse",
        "share/applications/glimpse.desktop",
        "share/icons/hicolor/scalable/apps/glimpse.svg",
        "install.py",
        "README.txt",
        *licenses,
    }
    if not required.issubset(manifest["files"]):
        raise ValueError("Package manifest is incomplete")
    for relative, expected in manifest["files"].items():
        if Path(relative).is_absolute() or ".." in Path(relative).parts:
            raise ValueError("Package manifest contains an invalid path")
        path = source / relative
        if hashlib.sha256(path.read_bytes()).hexdigest() != expected:
            raise ValueError(f"Package verification failed: {relative}")
    executable = prefix / "bin/glimpse"
    icon = prefix / "share/icons/hicolor/scalable/apps/glimpse.svg"
    desktop = prefix / "share/applications/glimpse.desktop"
    entry = (source / "share/applications/glimpse.desktop").read_text()
    entry = entry.replace(
        "Exec=glimpse --desktop -- %F\n",
        f"Exec={desktop_program(executable)} --desktop -- %F\n",
    )
    entry = entry.replace(
        "TryExec=glimpse\n", f"TryExec={desktop_string(str(executable))}\n"
    )
    entry = entry.replace("Icon=glimpse\n", f"Icon={desktop_string(str(icon))}\n")
    replacements = [
        (executable, (source / "bin/glimpse").read_bytes(), 0o755),
        (
            icon,
            (source / "share/icons/hicolor/scalable/apps/glimpse.svg").read_bytes(),
            0o644,
        ),
        (desktop, entry.encode(), 0o644),
        *((prefix / name, (source / name).read_bytes(), 0o644) for name in licenses),
    ]
    changed = []
    try:
        for destination, content, mode in replacements:
            destination.parent.mkdir(parents=True, exist_ok=True)
            backup = None
            if destination.exists() or destination.is_symlink():
                if destination.is_dir():
                    raise ValueError(
                        f"Installation destination is a directory: {destination}"
                    )
                backup = destination.with_name(
                    destination.name + f".previous-{time.time_ns()}"
                )
                shutil.copy2(destination, backup, follow_symlinks=False)
            temporary = None
            try:
                with tempfile.NamedTemporaryFile(
                    dir=destination.parent, delete=False
                ) as output:
                    temporary = Path(output.name)
                    output.write(content)
                    output.flush()
                    os.fchmod(output.fileno(), mode)
                    os.fsync(output.fileno())
                os.replace(temporary, destination)
                changed.append((destination, backup))
            finally:
                if temporary is not None:
                    temporary.unlink(missing_ok=True)
        if shutil.which("desktop-file-validate"):
            subprocess.run(["desktop-file-validate", str(desktop)], check=True)
        if shutil.which("update-desktop-database"):
            subprocess.run(["update-desktop-database", str(desktop.parent)], check=True)
    except BaseException:
        for destination, backup in reversed(changed):
            if backup is None:
                destination.unlink(missing_ok=True)
            else:
                os.replace(backup, destination)
        raise
    return executable, desktop


def main():
    parser = argparse.ArgumentParser(
        description="Install Glimpse without changing default file associations"
    )
    parser.add_argument("--prefix", type=Path, default=Path.home() / ".local")
    args = parser.parse_args()
    prefix = args.prefix.expanduser().resolve()
    try:
        executable, desktop = install(Path(__file__).resolve().parent, prefix)
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Installation failed: {error}\n")
    print(f"Installed {executable}\nDesktop entry: {desktop}")


if __name__ == "__main__":
    main()
