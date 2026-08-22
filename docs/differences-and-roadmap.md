# DevScribe: roadmap

## Phased plan for remaining work

Sequencing rationale: multi-tab support touches the data model that
`json_view.rs`, `diff_view.rs`, `shell.rs::code_area()`, LSP lifecycle, and
the tab bar all currently assume ("exactly one open file") — doing it once,
first, avoids re-touching those same files again in a later phase. Git
status is the next highest-value, self-contained feature. Visual polish and
the settings restructure are lower-risk and can follow once the tab model is
stable. Multi-project support and the Assist panel are large enough to stay
explicitly deferred pending a product decision.

### Phase 1 — Multi-tab editing (foundational) — ✅ done

Replaced `Tab` + `State.editor: Option<EditorState>` with a real open-tabs
list. Landed shape (differs slightly from the original sketch — a content-
keyed identity turned out simpler than a separate id counter):

- `State.open_tabs: Vec<OpenTab>` + `active_tab: Option<TabKey>`, where
  `TabKey` (`File(PathBuf)` / `Diff(PathBuf)` / `Search`) doubles as both the
  tab's identity *and* its dedup key — no separate id counter needed, since
  a path (or `Search`, a singleton) is already a unique, stable key. Closing
  a tab looks up by key, not by index, so it can't silently refocus the
  wrong tab.
- `enum OpenTab { File(Box<EditorState>), Diff(PathBuf), Search }` — `File`
  is boxed per Clippy's `large_enum_variant` (an `EditorState` is ~336
  bytes; boxing keeps `Diff`/`Search` from paying for space they don't use).
  A `Diff` tab carries only a path — its `DiffStatus` is read from the
  matching `File` tab's `EditorState.diff` at render time, so `open_or_focus_diff`
  always ensures a backing `File` tab exists first (opening one if needed).
  `.json`-ness stayed an in-place view switch on `File`, as planned.
- New `Message`s: `SelectOpenTab(TabKey)`, `CloseTab(TabKey)`,
  `CloseActiveTab`, `OpenSearchTab`. `SelectFile`, `SearchResultSelected`,
  and the palette's file-open entry all route through `open_or_focus_file`
  (open-or-focus by path) — this is what makes opening a *second* `.rs` file
  additive instead of a replace, while still not duplicating a tab for a
  file already open. `open_or_focus_diff` and `open_or_focus_search` do the
  same for their kinds. (`OpenDiffTab` as a standalone `Message` was sketched
  but dropped — nothing needed to send it yet, since the only current
  diff-opening trigger is the palette's "View Diff" entry, which calls
  `open_or_focus_diff` directly; Phase 2's changes-panel will be the second
  trigger and can reintroduce it then.)
- Keyboard: Ctrl/Cmd+W closes the active tab, Ctrl/Cmd+Shift+F opens/focuses
  the search tab — both new, filling gaps the fixed-4-tabs model didn't have.
- LSP: `didOpen` fires once per file tab actually created (replayed for every
  open file if the server becomes ready after files were already opened),
  `didClose` once when a tab actually closes. `LspEvent::Diagnostics` routes
  to the matching open file by converting its `Url` back to a path
  (`uri.to_file_path()`) and looking it up, instead of the single `state.editor`.
- Kept the existing "recompute highlights/diagnostics/json/diff eagerly on
  every edit" philosophy per tab rather than going lazy, as planned.
- `tab_bar.rs` now renders `state.open_tabs` dynamically, each tab a
  select-button + separate close-button pair (not a nested button, which
  iced doesn't support) sharing one active/hover-styled underline.
- `json_view::view` and `diff_view::view` now take an already-resolved
  `&EditorState` (json) or `(&State, &Path)` (diff) instead of pulling
  `state.editor` themselves; `shell.rs::code_area` takes `&EditorState`
  too. `status_bar.rs`'s problem count now sums diagnostics across *all*
  open files, not just the active one — diagnostics are workspace-wide and
  every open file keeps getting `didChange`d even while unfocused.
- Covered by new unit tests in `state.rs` (`open_or_focus_file` dedup/
  additive behavior, closing a file tab also closing its orphaned diff tab
  and refocusing correctly, closing the last tab clearing `active_tab`) —
  the refocus-after-close test caught a real bug during implementation
  (closing a `File` tab whose `Diff` tab was the *active* one didn't refocus,
  since the old logic only checked whether the explicitly-closed key matched
  `active_tab`, not tabs removed as a side effect).

### Phase 2 — Real git status: Changes panel + file-tree badges — ✅ done

- `devscribe_core::git::Repo::changed_files()` landed for real, backed by
  `gix::Repository::status()`. Turned out to need three `gix` API layers,
  not one: `Item::TreeIndex(gix_diff::index::ChangeRef)` for staged
  changes vs. `HEAD` (Addition/Deletion/Modification), and
  `Item::IndexWorktree(index_worktree::Item)` for unstaged changes
  (`Modification { status: EntryStatus<Change> }`, where `Change::Removed`
  means deleted-from-worktree) plus untracked files
  (`DirectoryContents`). The two passes are merged into one
  `BTreeMap<PathBuf, ChangeKind>`, worktree state overwriting a staged-only
  classification where both exist, since worktree state is more current.
  Rename/copy tracking is left at its default (off) — a rename shows up as
  a plain add+delete pair, an acceptable simplification for a one-letter
  badge that was never going to show rename provenance anyway. Explicitly
  set `untracked_files(UntrackedFiles::Files)` rather than the default
  `Collapsed`, since a collapsed brand-new directory would report one
  status for the whole directory instead of one per file, and the sidebar
  needs a badge per file. Verified with a real test (not just "it
  compiles"): `git::tests::detects_modified_added_untracked_and_deleted`
  drives the actual `git` CLI to build a fixture repo (init, commit, then
  modify/delete/add/untrack files) and asserts `changed_files()` classifies
  each one correctly — the only place in the test suite that shells out,
  and only as fixture setup; `changed_files()` itself stays pure `gix`.
- Per-file insertion/deletion counts: `compute_changed_files` in
  `devscribe/src/state.rs` reuses `Repo::head_text` (HEAD's blob, empty for
  new files) against the file's current *on-disk* content via
  `fs::read_to_string` (empty for deleted files, so a deletion counts as
  all-removed lines) and feeds both into the existing
  `devscribe_core::diff::diff_lines`. Deliberately reads on-disk content
  rather than the live buffer, unlike the per-tab `DiffStatus` in
  `recompute_diff_for` — the Changes panel covers the whole working tree,
  including files that aren't even open.
- Recompute timing: no file-watcher exists (same limitation the file tree
  already had), so `refresh_changed_files` only runs at startup and after
  `EditorSave` — the one internal action guaranteed to change the working
  tree. External edits (outside DevScribe) won't be reflected until the
  next save.
- Sidebar: a collapsible "CHANGES [n]" section (`sidebar.rs::changes_section`)
  between the project switcher and the file tree, with aggregate +/− in the
  header and a per-file row (name, +/−, single-letter M/A/D/U badge colored
  via the palette's status colors — warn/ok/danger/info respectively).
  Hidden entirely when there are no changes, rather than always showing an
  empty "CHANGES [0]" — avoids clutter on a clean tree. Clicking a row opens
  (or focuses) that file's Diff tab via the new `Message::OpenDiffFor`.
- File tree: each `Node::File` row gets the same single-letter badge
  (`sidebar.rs::kind_letter`, shared with the Changes panel) when
  `state.changed_files` has an entry for its path, looked up via a
  `HashMap<PathBuf, ChangeKind>` built once per `view()` call and threaded
  through the tree's recursion. Simplified from the original "dirty dot +
  badge" phrasing to just the badge — a separate dot next to a badge that
  already encodes "this file changed" would be redundant, not additional
  information.

### Phase 3 — Visual polish — ✅ done

- Fixed the editor gutter background: removed its separate `bg_panel` fill
  entirely rather than repainting it `bg_void` — the container behind the
  canvas already paints `bg_void`, so a matching second fill was a redundant
  draw, not just the wrong color.
- Highlighted the cursor's current line: a full-width (gutter to right edge),
  8%-alpha `accent` tint drawn first each line so selection/text/caret still
  render on top of it, matching the mockup's `color-mix(accent 8%)` line.
- Restyled `search_view.rs` to match the mockup: results grouped by file
  under a per-file header (lang badge, name, match count), a stats line
  ("N MATCHES // M FILES // K MS" — timing now measured in `recompute_search`
  via `Instant`), and each match line syntax-colored with the matched term
  highlighted. The syntax coloring reuses the *same* `Highlighter` and
  `editor_canvas::highlight_color` mapping the editor uses — run once per
  file with results, not once per match, then sliced per matched line the
  same way `editor_canvas.rs` slices highlight spans per visible line.
  Case-sensitivity/regex toggles were left out, as flagged — no real modes
  to back them yet.
- Command palette: added a footer hint row and a canvas-drawn search-icon +
  "Commands" mode badge in the header (`ui/search_icon.rs`, same
  draw-don't-load-an-asset pattern as `logo_mark.rs`, so it recolors with
  the theme).
- Fixed the active tab's background not filling the full tab (screenshot-
  confirmed bug): `tab_bar.rs::tab_shell` now wraps the select-button and
  close-button in one shared `container` that paints the active background
  for the whole tab, with both inner buttons rendering transparent except
  for their own hover feedback.
- Fixed a second, related tab-bar bug found later (also screenshot-
  confirmed): the active/inactive underline strip beneath each tab was a
  `Length::Fill`-width `Space` stacked below the tab's content in a
  `column!`. A `Fill` child inside a `Shrink`-sized column still expands to
  whatever max-width limit its ancestor `Row` hands it — with one tab as the
  last/only item in the bar, that generous limit was effectively "the rest
  of the tab bar," so the underline (and the column carrying it) stretched
  all the way to the overflow button instead of tracking the tab's own
  width. Replaced with `with_underline()`: wraps the tab's content in a
  `container` with 1px of bottom padding and the indicator color as that
  container's *background* — the 1px strip shows through only where the
  content doesn't cover it, so the container's width is simply the
  content's natural width, with no `Fill` anywhere in the tree.
- Along the way, fixed a real latent bug in `devscribe_core::search`:
  `SearchHit::preview` was `.trim()`-med (both ends) while `col` was measured
  against the *untrimmed* original line — correct for placing the cursor on
  "Enter", but silently wrong for indexing into `preview` (which the new
  match-highlighting needed to do), since leading-whitespace trimming shifts
  every offset after it. Fixed by trimming only the trailing end, so `col`
  indexes correctly into both the original line and `preview` with one
  offset instead of two that happened to agree only when there was no
  leading whitespace. Covered by a new test
  (`col_indexes_correctly_into_preview`) exercising a line with leading
  tabs/spaces.
- New tests: `devscribe-core`'s `search::tests::col_indexes_correctly_into_preview`;
  `devscribe`'s `state::tests::line_segments_matches_real_highlighter_output`
  (verifies the search-preview segment-slicing against a real `Highlighter`
  run, not synthetic spans).

### Phase 3.5 — Pinned search icon + in-file find (Ctrl+F) — ✅ done

Not one of the original 5 phases — added mid-stream from a direct request
prompted by a reference screenshot of a VS-Code-style find widget. Two parts:

- **Pinned search icon**: `OpenTab::Search` removed entirely — the project
  search view is no longer a dynamic, closeable tab-list entry. `tab_bar.rs`
  now renders a fixed, icon-only 36px button (`search_icon_tab`, reusing
  `search_icon.rs`) before the dynamic tab list, matching the mockup's
  original "always there, not a tab you close" treatment.
  `PaletteAction::FocusSearchTab` / `Message::FocusSearchTab` (renamed from
  `OpenSearchTab`) now just sets `active_tab`, no `open_tabs` push.
- **In-file find (Ctrl+F)**: new, separate from project-wide search
  (Ctrl+Shift+F, unchanged). `EditorState.find: Option<FindState>`
  (`query`/`matches`/`current`) — per-tab state, like `highlights`/`json`,
  recomputed via `refind()` alongside `rehighlight()`/`reparse_json()` on
  every edit. A floating pill (`ui/find_bar.rs`) overlays the top-right of
  the editor pane via a local `stack!` in `shell.rs::code_area` when
  `find.is_some()` — search icon, text input (auto-focused via
  `operation::focus`), a "N of M" counter, up/down buttons, and a close
  button. Matches are highlighted directly in `editor_canvas.rs`'s per-line
  draw loop (same char-range-to-rect conversion the selection highlight
  already used, factored out so both share it): a subtle tint for every
  match, a stronger tint + border for the current one. Ctrl+F toggles the
  bar for the active file (seeding the query from the current selection,
  if any); Enter/↓ = next match, ↑ = previous, wrapping; Escape closes find
  before falling through to palette/settings.
  Deliberately **not** built: the mockup screenshot's `Aa`/`ab`/`.*`
  case-sensitive/whole-word/regex mode toggles (no such modes exist in
  `devscribe_core::search` — same "no fake controls" call already made for
  the project-search panel in Phase 3) and auto-scroll-to-match (the code
  area's `scrollable` has no stable `.id()` wired up yet; navigating to a
  match outside the current viewport requires manually scrolling — a known,
  minor gap, not a decided-against feature).

### Phase 4 — Settings panel restructure — ✅ done

- Left-nav modal shell (`settings_panel.rs`), fixed at the mockup's 760×520:
  a `SettingsCategory` enum (Explorer/Editor/Toolchains/Keymap/Advanced,
  new `state.settings_category`, sticky across opens/closes) drives a left
  nav column and a right content pane split by a `widgets::vline` (new —
  the sidebar only had a horizontal-hairline `hline` before).
  Toolchains/Keymap/Advanced render an honest "settings aren't available
  yet" placeholder rather than empty space that looks broken or, worse, a
  switch that would do nothing.
- Content split to match the mockup's own categorization (confirmed via
  the earlier design-diff pass): **Explorer** gets everything that isn't
  specific to the code editor's own text — Theme, Row density, UI text
  size, and the two toggles below — while **Editor** ends up holding just
  the editor font-size stepper. This moves "inline problem hints" out of
  DevScribe's old flat panel, where it sat under an "Editor" label that
  didn't match the mockup's placement.
- Added the third density tier, "Spacious" (`density.rs`) — sized above
  Comfortable (46px chrome rows / 34px sidebar rows vs. 38px / 28px),
  making Compact/Comfortable/Spacious a real low-to-high progression rather
  than just two points.
- Added "Show git status in tree" (`state.git_status_in_tree`, default on),
  wired to Phase 2's per-file tree badges: `sidebar.rs::view` now builds an
  empty lookup map instead of the real one when the toggle is off, so
  `node_view` doesn't need to know the setting exists at all. Deliberately
  left the separate "CHANGES" panel unaffected — it already has its own
  collapse toggle, and the mockup's switch is specifically about the
  per-file tree markers.
- Did **not** add "Install toolchains automatically" — no toolchain
  installer exists in DevScribe; a switch for it would be a fake feature,
  consistent with every other "no fake controls" call made earlier in this
  doc (search mode toggles, find case/regex toggles).
- Found and fixed one unrelated pre-existing syntax error while getting a
  clean baseline build (`devscribe/src/color.rs`: a stray `ss` after a
  closing brace was breaking the whole workspace build) — not part of this
  phase's scope, just a blocker in the way of it.

### Phase 5 — Explorer & tab-bar quality-of-life — ✅ done

- **Inline drafts + tree context menu (item 12)**: `state.rs` gained
  `Draft`/`DraftKind` (`NewFile`/`NewFolder`/`Rename`) and `State.draft:
  Option<Draft>`. Unlike the mockup — where committing a new-file draft
  opens a fake "unsaved buffer" tab (item 14, still deferred to Phase 9) —
  DevScribe's commit is real end-to-end: `commit_draft` writes an empty file
  (or directory) to disk immediately, re-walks the tree (`refresh_tree`),
  and opens the new file as a real tab via the existing
  `open_or_focus_file`. Rename is a real `std::fs::rename` too, with a new
  `rename_open_tab` that repoints an already-open tab's `EditorState.path`
  *and* its `Document`'s own path (a new `Document::set_path` in
  `devscribe-core` — added because `Document::save()` writes to its
  internal path, which a rename would otherwise leave pointing at the
  now-deleted old file) in place, re-notifying the LSP server
  (`didClose`/`didOpen`) rather than closing and reopening the tab, which
  would have thrown away unsaved edits and cursor position. `sidebar.rs`'s
  `node_view` threads a new `TreeCtx` bundle (added to stay under clippy's
  argument-count lint once `draft` became a fifth read-only lookup) and
  renders the draft as an inline `text_input` row — inserted above a
  directory's children for `NewFile`/`NewFolder`, or standing in for an
  existing row's own button for `Rename`. Right-click wiring uses
  `mouse_area::on_right_press` per row (confirmed safe against the existing
  per-row `button`: `button` only claims the *left*-click event, so a
  right-click always reaches the wrapping `mouse_area` untouched) plus one
  more wrapping the whole tree for right-clicks that miss every row
  (targets the project root). The context menu itself
  (`ui/context_menu.rs`) is a **deliberate, documented divergence** from the
  mockup: it anchors to a fixed spot near the top of the sidebar rather than
  the exact cursor pixel. `iced` 0.14's `mouse_area::on_right_press` takes a
  fixed `Message`, not a `Fn(Point) -> Message`, so there's no cursor
  position to position against — and getting the *real* row position would
  additionally need the tree `scrollable`'s live scroll offset, which isn't
  tracked anywhere (same class of gap as Ctrl+F's already-documented
  auto-scroll-to-match). The menu's header names its target so this stays
  unambiguous. "Collapse all" is real, not decorative like the mockup's own
  version (`collapseAll: () => this.flash(...)`, no actual state change) —
  it populates `collapsed_dirs` from `fs_tree::flatten_dirs`, the same
  helper that seeds the tree's default-collapsed startup state.
- **Tab-bar overflow menu (item 16)**: `tab_bar::overflow_menu` renders
  Close others / Reveal in tree / Reopen closed tab as a top-level `stack!`
  layer (same backdrop-click-closes pattern as `settings_panel`/
  `command_palette`), positioned precisely under the `⋯` button (unlike the
  context menu, this one *is* exact — it's anchored to a fixed chrome
  element, not a cursor position, so `state.density.title_bar_h() +
  tab_bar_h()` is all the math it needs). Split right is omitted entirely,
  not rendered-but-inert — per the roadmap's item 16 framing that ties it to
  item 7's window-control-buttons precedent, and item 7 says DevScribe
  dropped those *entirely* on request, not decoratively kept them. Close
  others (`close_other_tabs`) and Reopen closed tab
  (`reopen_closed_tab`/`State.closed_tabs`, a capped LIFO stack pushed to
  from the one `close_tab` choke-point so every close path — explicit
  `CloseTab`, `CloseActiveTab`, and the new `CloseOtherTabs` — feeds it for
  free) both reuse `close_tab`/`open_or_focus_file` rather than hand-rolling
  tab removal, so LSP notify and diff-tab cleanup keep working. Reveal in
  tree (`reveal_active_in_tree`) only does the achievable real part —
  expanding every ancestor directory of the active file — and doesn't
  attempt to scroll to it, same documented gap as the context-menu
  positioning above. New keyboard shortcuts: ⌘N/⇧⌘N (new file/folder, root-
  targeted), ⌥⌘W (close others, vs. plain ⌘W for close-active), ⇧⌘E
  (reveal), ⇧⌘T (reopen).
- **Flash pill (item 21)**: a new `Flash`/`State.flash` and `ui/flash.rs`,
  center-bottom, explicitly *not* built on top of the existing `Toast`
  stack (`ui/toast.rs`) per the roadmap's own framing — separate struct,
  separate render layer, pruned by the same 1-second `PruneToasts` tick
  rather than a dedicated timer (coarser than the mockup's 1.8s exact
  timeout, an accepted trade-off against adding a second subscription for
  one pill). Fires from exactly the five actions the roadmap named:
  new-file/new-folder/rename commit, copy path, collapse all — Close
  others/Reveal in tree/Reopen closed tab deliberately do *not* flash,
  matching the roadmap's precise "fired by commitDraft/copyPath/
  collapseAll/saveFileAction" list (and `saveFileAction` itself — the
  "More actions ⋯ → Save" menu item — is part of item 13, still deferred to
  Phase 9, so `⌘S` keeps using the pre-existing toast, unchanged).
- **Styled empty tab-bar state (item 22)**: `shell.rs`'s `no_buffer_state`
  replaces the plain placeholder only when `active_tab` is genuinely `None`
  — the defensive fallback for an `active_tab` pointing at a missing editor
  (shouldn't normally happen) keeps the old plain `widgets::placeholder`.
- New tests in `state.rs`: `commit_draft_new_file_writes_to_disk_and_opens_tab`,
  `commit_draft_rename_updates_open_tab_path_in_place` (catches exactly the
  "document path left stale after rename" bug class `Document::set_path`
  exists to prevent), `close_other_tabs_keeps_only_the_active_tab`,
  `reopen_closed_tab_restores_the_most_recently_closed`,
  `reveal_active_in_tree_uncollapses_ancestor_dirs`.
- **Known gap, same class as Ctrl+F's**: neither the context menu nor
  "Reveal in tree" scroll the sidebar tree to the target — `sidebar.rs`'s
  tree `scrollable` still has no stable `.id()` wired up. Not attempted
  here; flagged for whoever eventually wires scroll-to-id, likely alongside
  the Ctrl+F gap since it's the same missing piece.

### Phase 6 — Command palette & working-tree-diff shortcut — not started

- Contextual "Diff:" command grouping + FILES section when the query matches
  "diff" (item 17).
- Global `⇧⌘D` working-tree-diff command, independent of the palette grouping
  above (item 18).

### Phase 7 — Settings panel v2 — not started

- Real Toolchains status list, Shortcuts (renamed Keymap) static reference
  table, new About category, drop Advanced entirely (item 19). Needs a
  product decision on the Editor category's font-size stepper before
  landing — keep it (diverging from the mockup) or drop it to match.

### Phase 8 — Branch ahead/behind counts — not started

- Sidebar `▲2 ▼0` indicator next to the branch name (item 20).

### Phase 9 — Deferred, needs a product decision first

- Multi-project switcher, rescoped: needs a native folder-picker dependency
  (no `rfd`-equivalent crate exists yet) and a real "recent projects"
  concept, not just UI (items 13, 15).
- Unsaved/"untitled" file buffers + a real "write new file to disk" save
  path — a data-model addition on par with Phase 1's tab-model work, not
  just UI (item 14).
- Window control buttons / logo: intentionally not revisited unless the
  user changes their earlier decision.

### Phase 10 — Assist (AI chat) panel — deliberately last

Milestone 7 in full: docked / collapsed-rail / floating-window modes (the
popped-out window mode renders as a distinct, differently-styled chat
surface, not just a repositioned docked panel), Claude API streaming chat,
inline diff-suggestion cards with Apply/Open-diff actions, `@file`/`#symbol`/
`/command` input affordances. Large, separately-scoped feature. **Kept
explicitly as the last phase per direct instruction** — every phase above
(5 through 9) lands first, regardless of relative size or difficulty.

## Verification approach (per phase)

Same as every prior milestone in this project: `cargo build`, `cargo clippy
--workspace --all-targets`, `cargo test --workspace`, and a short smoke-run
of the binary after each phase. Visual/interactive claims (tab switching,
panel layout, hover states) can't be screenshotted in this sandbox — those
need a manual check by the user after each phase lands.
