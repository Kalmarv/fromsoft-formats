# Testing

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
```

These need no game files: the unit tests and the tests on hand-built fixtures run everywhere, and the tests that read a real install skip with a line naming the missing input. `cargo test -p fromsoft-formats --doc` checks the crate documentation, and `cargo build -p fromsoft-formats --examples` builds the example programs. Examples that read paths need your own files; the repository ships no game assets.

## Real-data tests

To run the real-data tests against your own copies, write a TOML file and point the `GAME_DATA` environment variable at it. Every section and key is optional. Paths below are placeholders:

```toml
[installs]
sekiro = 'D:/placeholder/Sekiro'
elden = 'D:/placeholder/ELDEN RING/Game'
[keys]                                   # folders of the PEM files you hold
sekiro = 'D:/placeholder/keys/sekiro'    # Data1.pem ... Data5.pem
elden = 'D:/placeholder/keys/elden'      # Data0.pem ... Data3.pem, DLC.pem, sd.pem, sd_dlc02.pem, regulation.key
[tools]
oodle6 = 'D:/placeholder/Sekiro/oo2core_6_win64.dll'
[oracles]
root = 'D:/placeholder/oracles'          # reference output made by other readers, see below
[scratch]
root = 'D:/placeholder/scratch'
[run]
require = 'gate'                         # a missing gate input fails the test instead of skipping it
gate = 'smoke'                           # or 'full': how much the heavy tests cover
```

[tests/inputs/README.md](../tests/inputs/README.md) lists every input the tests read, what it is and where it comes from. Besides the installs, keys and Oodle library, some tests compare this crate's reading with the output of independent readers (SoulsFormats, SoulsFormatsNEXT, the fxr library, UXM's path lists) kept under `oracles.root`; those files are made by the user from their own installs and are not part of this repository. The file layout comes from [primase-testdata](https://github.com/Kalmarv/primase-testdata), which also has a sample.

The `census` example runs the FLVER and effects censuses against the same configuration and keeps each report to list what changed since the last run:

```sh
cargo run --release --example census -- flver sekiro elden
```
