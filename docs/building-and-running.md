# Building and running DevScribe

DevScribe is a Rust workspace with two crates:

- `devscribe-core` — library crate: text buffer, syntax highlighting, git diffing, search, and LSP client logic.
- `devscribe` — binary crate: the `iced`-based desktop UI, built on top of `devscribe-core`.

## Prerequisites

- `rustc`/`cargo` 1.96.

## Build

From the repo root (workspace):

```sh
cargo build            # debug build
cargo build --release  # optimized build
```

This builds both workspace members. Build artifacts land in `target/debug` or `target/release`.

## Run

```sh
cargo run -p devscribe            # debug
cargo run -p devscribe --release  # release
```

`devscribe` is the only binary in the workspace, so `cargo run` (without `-p`) also works from the repo root.

## Notes

- Fonts and icons are bundled under `devscribe/assets/` and embedded via `include_bytes!`/`iced` font loading — no separate asset install step.
- There is no CI configuration in this repo yet; the above commands are also what you'd wire into one.
