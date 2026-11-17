#!/usr/bin/env python3
import gzip
import hashlib
import json
import os
import platform
import shutil
import subprocess
import tarfile
import tempfile
import tomllib
from pathlib import Path


def main():
    if platform.system() != "Linux":
        raise SystemExit("The Linux archive must be packaged on Linux")
    root = Path(__file__).resolve().parent.parent
    subprocess.run(["cargo", "build", "--release", "--locked"], cwd=root, check=True)
    version = tomllib.loads((root / "Cargo.toml").read_text())["package"]["version"]
    name = f"glimpse-{version}-linux-{platform.machine()}"
    output = root / "build"
    output.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(
        prefix=".glimpse-package-", dir=output
    ) as temporary:
        stage = Path(temporary) / name
        files = {
            "bin/glimpse": root / "target/release/glimpse",
            "share/applications/glimpse.desktop": root
            / "packaging/linux/glimpse.desktop",
            "share/icons/hicolor/scalable/apps/glimpse.svg": root
            / "packaging/linux/glimpse.svg",
            "install.py": root / "packaging/linux/install.py",
        }
        for relative, source in files.items():
            destination = stage / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, destination)
            destination.chmod(
                0o755 if relative in ("bin/glimpse", "install.py") else 0o644
            )
        (stage / "README.txt").write_text(
            "Glimpse for Linux\n\n"
            'Install: python3 install.py --prefix "$HOME/.local"\n'
            "Run: ~/.local/bin/glimpse [PATH ...]\n"
            "The installer registers an application and preserves previous installed files.\n"
            "It does not change your default file associations.\n\n"
            "Requires a compatible Linux desktop, graphics driver, and system libraries.\n"
            "Python 3 is needed only for installation. The application works offline.\n"
        )
        manifest = {
            "version": version,
            "platform": platform.machine(),
            "source_commit": subprocess.check_output(
                ["git", "rev-parse", "HEAD"], cwd=root, text=True
            ).strip(),
            "source_dirty": bool(
                subprocess.check_output(["git", "status", "--porcelain"], cwd=root)
            ),
            "features": [],
            "files": {
                str(path.relative_to(stage)): hashlib.sha256(
                    path.read_bytes()
                ).hexdigest()
                for path in sorted(stage.rglob("*"))
                if path.is_file()
            },
        }
        (stage / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
        archive = output / (name + ".tar.gz")
        staged_archive = Path(temporary) / "archive.tar.gz"
        with (
            staged_archive.open("wb") as raw,
            gzip.GzipFile(filename="", fileobj=raw, mode="wb", mtime=0) as compressed,
            tarfile.open(fileobj=compressed, mode="w|") as tar,
        ):
            tar.add(stage, arcname=name)
        staged_archive.chmod(0o644)
        os.replace(staged_archive, archive)
        print(f"{archive}\n{archive.stat().st_size} bytes compressed")


if __name__ == "__main__":
    main()
