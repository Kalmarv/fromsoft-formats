# Contributing

## Build and test

The toolchain is pinned in `rust-toolchain.toml`. From a clone:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
```

All four must pass; CI runs them on Linux and Windows. None needs game files: a test that reads a real install, key
folder or reference output skips with one line naming the missing input. To run those against your own copies, see
"Tests" in the [README](README.md) and [tests/inputs/README.md](tests/inputs/README.md).

## Changing a reader

- A reader takes the bytes of one file and returns what the file stores. It refuses what it does not understand with
  `Error::Invalid { format, offset, detail }` and does not guess, skip or panic. Add a test that feeds it a truncated or
  altered file and checks the error.
- Check each length, count and offset against the data before following it; use the helpers in `src/bytes.rs`
  (`View`, `Budget`, `coverage`) rather than indexing.
  `src/lib.rs` denies `clippy::indexing_slicing`, `clippy::arithmetic_side_effects` and
  `clippy::cast_possible_truncation` outside tests: add offsets with `View::at` and `View::element`, read with `get`,
  convert with `try_from`, and give an allocation sized from the file a bound (`Budget`, or a table slice) first.
- Only `src/archive.rs` and `src/game_params.rs` read files from disk, and no library code reads the environment or
  starts a process; `clippy.toml` and `#![deny(clippy::disallowed_methods)]` enforce it.
- A field whose meaning is not established keeps its stored value under a name that says where it sits
  (`unknown_30`), not an invented meaning.
- A new format or game-specific rule needs a doc comment saying which versions it reads, and a test on a hand-built
  file that does not need game data.
- If a test reads real data, declare the input in `tests/inputs/mod.rs`, then update the generated tables in
  `tests/inputs/README.md` (the `real_data_inputs` test prints the expected text when they differ).

Commit messages say what changed and why. Do not add game files, keys, dictionaries or native libraries to the
repository, in tests or fixtures: fixtures are built by hand in the test.

## Reporting a file it cannot read

Open an issue with the "File this crate cannot read" template. Say which game and version it is, which file (its
archive path or name), which function you called and the full error message, which names the format and the offset.

**Attach:** the error text, the offset it names, the file's size and the output of `xxd -s <offset - 32> -l 96` (or the
equivalent) around that offset, and the SHA-256 of the file so it can be matched to another copy of the game.

**Do not attach:** the game's files (or any archive, binder or the file itself), archive keys, `regulation.key`, the
Oodle library, or a path dictionary. A few dozen bytes around the offset are enough to see what the reader met; if
more is needed you will be asked for a specific range. A file you made yourself from scratch (a hand-built minimal one
that shows the problem) is welcome.
