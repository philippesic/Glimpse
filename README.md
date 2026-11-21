# Glimpse

Glimpse is a native file navigator and UTF-8 text editor. Browse folders and preview files in one window, then edit, search, or switch a Markdown file to a rendered reading view.

- Open a folder, navigate with the keyboard, and keep files in tabs as you move between folders.
- Find filenames or search saved text recursively. Pin the search folder to keep its scope while navigating.
- Edit UTF-8 files with syntax highlighting, find and replace, and undo. Review external changes before overwriting them and recover unsaved work after an interruption.
- Use the command line to open a file at a line and column, wait for an editing request to finish, or select a path for another command.

## Try Glimpse on Linux

Glimpse is in early development. Linux is the current desktop target. See the [qualification status](docs/qualification.md) for exercised workflows and remaining checks.

Check [GitHub Releases](https://github.com/philippesic/Glimpse/releases) for Linux preview builds. Download the archive for your CPU architecture (`uname -m`), extract it, then run these commands from the extracted directory:

```sh
python3 install.py --prefix "$HOME/.local"
"$HOME/.local/bin/glimpse" /path/to/folder
```

Preview archives are built on Ubuntu 24.04 for x86-64 and ARM64. Glimpse needs a Wayland or X11 desktop and a Vulkan driver. Native file dialogs use the desktop portal. On Ubuntu, install the runtime libraries if needed:

```sh
sudo apt install fontconfig libfreetype6 libxcb1 libxkbcommon0 libxkbcommon-x11-0 libvulkan1
```

The installer adds Glimpse to the application menu, preserves existing installations as backups, and leaves default file associations in place.

See the [usage guide](docs/usage.md) for navigation, keyboard shortcuts, and command-line examples.

## Build from source

The Rust toolchain is pinned in [rust-toolchain.toml](rust-toolchain.toml). Install the native development dependencies for your distribution:

```sh
# Fedora
sudo dnf install fontconfig-devel freetype-devel libxcb-devel libxkbcommon-devel libxkbcommon-x11-devel wayland-devel vulkan-loader-devel

# Ubuntu
sudo apt install build-essential pkg-config libfontconfig1-dev libfreetype-dev libxcb1-dev libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libvulkan-dev
```

```sh
cargo run --release --locked -- /path/to/folder
```

The application uses GPUI. See the [development and release guide](docs/development.md) for checks and packaging, the [product design](glimpse.md) for intended behavior, and the [core design](docs/core-design.md) for implementation details.

## License

Glimpse is available under the [MIT License](LICENSE). Third-party code and assets retain their own licenses.
