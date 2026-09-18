# Missing features

## Tunings — highest ROI, minimal code

**1. Add a `[profile.release]`. There isn't one at all.**

The release binary is 60 MB unstripped, 49 MB stripped; `.text` alone is 26.5 MB, `.rodata` 13.9 MB. You're shipping a debug-symbol-laden, thin-LTO, 16-codegen-unit build and calling it lightweight.

```toml
[profile.release]
lto = "fat"
codegen-units = 1
strip = "symbols"
panic = "abort"
```

Expect ~49 MB → high-20s MB, plus a genuine speedup on the rope/tree-sitter hot paths. Zero code change — this is the single best effort-to-payoff item in the list.

**2. Get the project walk off the startup path.** `sidebar::startup()` (sidebar.rs:231) runs a full recursive `fs_tree::walk` **plus** `Repo::open` **plus** a `changed_files()` status walk before the first frame exists. On a big monorepo that's a blank window. You already have the backgrounded version — `start_loading_project` (sidebar.rs:726). Reuse it for auto-reopen: paint the shell with an empty tree, fill it in on `ProjectLoaded`.

**3. Honor `.gitignore`.** The only filter today is a 6-entry hardcoded `SKIP_DIRS` (watcher.rs:26). `build/`, `dist/`, `.venv`, `__pycache__`, `vendor/`, `.next` all get walked, watched, and searched. Worse — `show_hidden_files` disables `SKIP_DIRS` wholesale (fs_tree.rs:128), so flipping that setting starts scanning `.git`. A root+nested `.gitignore` parser is ~150 lines with no new deps.

**4. Parallelize project search.** `run_search` (sidebar.rs:598) is single-threaded over up to 3000 files, `read_to_string` each. One chunk per core with std threads is near-linear, no new deps. The `MAX_SEARCH_FILE_BYTES` / `MAX_SEARCH_FILES_SCANNED` caps are compensating for the serial scan; they'd bite far less often.

**5. Stop reallocating the buffer every settle.** `reparse_now` (editor.rs:680) does `document.text().to_string()` — a full copy of the file per debounce tick. Keep a reusable `String` on `EditorState` and refill it from `Rope::chunks()`; you still copy, but you stop churning a multi-MB allocation. Minor next to 1–4.

**6. Optional: feature-gate the grammars.** 13 statically linked tree-sitter grammars are most of that 13.9 MB `.rodata`. Cargo features (`lang-web`, `lang-systems`, …) let a lean build drop what it won't use.