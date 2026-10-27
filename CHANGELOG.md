# Changelog

## 0.1.0 (unreleased)

- First release.
- Readers return errors, never panic or wrap, on offsets and counts near the top of the address range; `clippy::indexing_slicing`, `clippy::arithmetic_side_effects` and `clippy::cast_possible_truncation` are denied in library code.
- `Binder::read_split_where` reads a split binder keeping only the files a caller names; the others are checked as `read_split` checks them and come back with no bytes.
