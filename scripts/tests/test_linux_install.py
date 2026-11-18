import hashlib
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "linux_install", ROOT / "packaging/linux/install.py"
)
INSTALLER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(INSTALLER)


class InstallSafety(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.source = self.root / "package"
        self.prefix = self.root / "installed"
        files = {
            "bin/glimpse": b"#!/bin/sh\nexit 0\n",
            "share/applications/glimpse.desktop": (
                ROOT / "packaging/linux/glimpse.desktop"
            ).read_bytes(),
            "share/icons/hicolor/scalable/apps/glimpse.svg": b"new icon",
            "install.py": b"installer fixture",
            "README.txt": b"readme fixture",
        }
        for relative, content in files.items():
            path = self.source / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(content)
        (self.source / "manifest.json").write_text(
            json.dumps(
                {
                    "files": {
                        name: hashlib.sha256(content).hexdigest()
                        for name, content in files.items()
                    }
                }
            )
        )

    def test_corruption_is_rejected_before_any_installation(self):
        (self.source / "bin/glimpse").write_bytes(b"damaged")
        with self.assertRaisesRegex(ValueError, "verification failed"):
            INSTALLER.install(self.source, self.prefix)
        self.assertFalse(self.prefix.exists())

    def test_failed_later_destination_restores_the_original_executable(self):
        original = self.prefix / "bin/glimpse"
        original.parent.mkdir(parents=True)
        original.write_bytes(b"personal executable")
        original.chmod(0o751)
        icon = self.prefix / "share/icons/hicolor/scalable/apps/glimpse.svg"
        icon.mkdir(parents=True)
        personal = icon / "personal.txt"
        personal.write_bytes(b"keep this directory")
        with self.assertRaisesRegex(ValueError, "destination is a directory"):
            INSTALLER.install(self.source, self.prefix)
        self.assertEqual(original.read_bytes(), b"personal executable")
        self.assertEqual(original.stat().st_mode & 0o777, 0o751)
        self.assertEqual(personal.read_bytes(), b"keep this directory")

    def test_replacing_a_symlink_preserves_its_target_and_previous_binding(self):
        target = self.root / "personal-program"
        target.write_bytes(b"personal target")
        executable = self.prefix / "bin/glimpse"
        executable.parent.mkdir(parents=True)
        executable.symlink_to(target)
        INSTALLER.install(self.source, self.prefix)
        self.assertEqual(target.read_bytes(), b"personal target")
        self.assertFalse(executable.is_symlink())
        backups = list(executable.parent.glob("glimpse.previous-*"))
        self.assertEqual(len(backups), 1)
        self.assertTrue(backups[0].is_symlink())
        self.assertEqual(backups[0].resolve(), target)

    def test_unsupported_desktop_path_is_rejected_before_writes(self):
        for name in ["cannot=represent", "cannot%represent"]:
            with self.subTest(name=name):
                prefix = self.root / name
                with self.assertRaisesRegex(ValueError, "Installation paths"):
                    INSTALLER.install(self.source, prefix)
                self.assertFalse(prefix.exists())

    def test_installed_desktop_launcher_preserves_restore_and_file_arguments(self):
        prefix = self.root / "installed with spaces"
        executable, desktop = INSTALLER.install(self.source, prefix)
        self.assertIn(f'Exec="{executable}" --desktop -- %F', desktop.read_text().splitlines())


if __name__ == "__main__":
    unittest.main()
