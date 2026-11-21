# Development and releases

See the [README](../README.md) for build dependencies. Use the commands in the [Check workflow](../.github/workflows/check.yml) to reproduce the platform checks. Linux filesystem tests need util-linux `unshare` and `mount`, with user and mount namespaces available. The bind-mount fixtures run in a private namespace. macOS compilation and packaging require Apple’s Command Line Tools.

For desktop testing, follow the [native evaluation guide](native-evaluation.md) and use an isolated `GLIMPSE_DATA_DIR`. See [qualification](qualification.md) for the current evidence and remaining checks.

## Package for Linux

Use Python 3.11 or later and install `cargo-about` 0.9.3. Run `python3 scripts/package-linux.py`. The archive appears under `build/` and includes the licenses and a manifest with the source commit and payload hashes.

Test the installer with `python3 -m unittest discover -s scripts/tests -v`. Install the archive into a disposable prefix and launch the installed executable on a native desktop to verify changes to packaging.

## Publish a Linux preview

1. Use the successful Check run for the exact commit on `main` intended for release. Download both Linux architecture artifacts and check that each archive’s manifest names that commit with `source_dirty` set to `false`.
2. Verify the installed app against the [native evaluation guide](native-evaluation.md) and document the tested configurations and remaining limitations. Check the licenses in the archives before distributing them.
3. Tag the commit with a version matching `Cargo.toml`, such as `v0.1.0`. Create a draft GitHub prerelease with both archives, a `SHA256SUMS` file, and notes covering installation, platform requirements, and known limitations. Verify the assets and tag before publishing.

## Trace desktop work

Set `GLIMPSE_TRACE=1` to record monotonic edit and save timings on stderr. Events identify documents and generations without logging their contents. Set `GPUI_WORK_TRACE=1` to record monotonic start times and durations in nanoseconds for UI updates, input dispatch, effects, frame construction and renderer submission. Merge overlapping intervals for nested work. The traces describe application handling and renderer submission. Measure physical presentation independently, and disable tracing for idle CPU measurements.
