# DevScribe: roadmap

**2026-08-22 update**: the Claude Design project's `DevScribe.dc.html` mockup
was refreshed. `_ds/axiom-hud-design-system-*/tokens/colors.css` (the four
themes) is byte-for-byte unchanged — nothing to do there. The mockup itself
picked up three things, folded into the plan below:

- A full "Select a project" **welcome/launcher screen** (`welcomeOpen`),
  shown before any project is loaded: a fixed left rail (build/toolchain
  info) plus a "Recent" project list (name, path, git branch or "no
  repository", change count, last-opened time), Open folder / Clone
  repository / New project buttons, and a "Loading workspace" overlay while
  a pick resolves. Absent before this refresh — the old mockup opened
  straight into a fixed demo project. Folded into Phase 9's multi-project
  bullet below, since it's a much fuller spec of the same already-deferred,
  same-blocker feature.
- The sidebar's project-switcher header can now show **"No repository"** (in
  place of the branch row, when the project isn't a git repo) and a
  **"Working tree clean"** row (in place of the CHANGES section, when it's a
  git repo with zero changes) — previously that header only ever showed a
  hardcoded branch name. Unlike the welcome screen, this needs no new
  dependency: `state.repo`/`state.changed_files` already carry exactly the
  data needed. Folded into Phase 8 below.
- A new "Close project" row in the project-switcher dropdown, returning to
  the welcome screen — blocked on the welcome screen existing at all, so it
  travels with it in Phase 9.

**2026-08-25 update**: the Claude Design project was pointed at a whole new
design system, **Maho**, replacing Axiom HUD entirely (not just a token
tweak — `DevScribe.dc.html` no longer references
`_ds/axiom-hud-design-system-*` at all). Adopted in full, on request, rather
than the narrower "just make the spacing bigger" option also on the table:

- **Palette**: Axiom HUD's ten fixed named themes (NullGrid, Gantry,
  Abyssal, Raven, Ember, Verdigris, Meridian, Stark, Sumi, Washi) are gone.
  `devscribe_core::theme` now has `ThemeMode` (`Dark`/`Light`, Maho's
  "Yoru"/"Asa") crossed with `Accent` (`Tsuki`/`Seiji`/`Matcha`/`Fuji`/
  `Kohaku`/`Nezu` — six of the mockup's nine defined accent ramps are
  exposed, matching the mockup's own curated picker; `Sakura`/`Shu`/`Wakaba`
  exist in the source CSS but aren't offered there either). `Palette`'s
  field names changed to Maho's own semantics throughout (`bg_void`→
  `bg_canvas`, `text_primary`→`text_strong`, `line_neutral`→
  `border_hairline`, `accent`→`accent_solid`, `status_ok`→`status_success`,
  etc.) — a few fields with zero real call sites (`accent_dim`,
  `accent_glow`, `grid_line`/`grid_dot`, the `status_*_soft` variants) were
  dropped rather than carried forward as dead weight. The old `accent_2`
  (a second brand color, used for the title-bar logo's second half and a
  couple of syntax-highlight roles) maps onto Maho's `seal_solid` — a fixed
  vermilion "used once per view at most" per its own source comment, which
  is exactly that role.
- **Settings/persistence**: `settings.json` now stores `theme_mode` +
  `accent` instead of one `theme` key. An old, pre-Maho settings file
  (`{"theme":"NullGrid"}`) simply fails to parse into the new shape and
  falls back to the default (`Dark`/`Tsuki`) — same "nothing saved yet, use
  the default" treatment every other unrecognized/missing key already got,
  no migration code needed or added.
- **Settings panel / command palette**: the old ten-button theme grid is
  now two pickers — a Dark/Light toggle and a six-swatch accent row — and
  the command palette's ten "Theme: X" entries became two "Theme:
  Dark/Light" entries plus six "Accent: X" entries.
- **Fonts**: the three-family stack (Oxanium display / Archivo sans / IBM
  Plex Mono) is gone — Maho is one monospaced voice, so all three roles
  (`fonts::display`/`sans`/`mono`) now resolve to the same bundled
  JetBrains Mono (instanced at the weights the app actually uses: Light/
  Regular/Medium/SemiBold/Bold/ExtraBold, no italic — nothing renders one).
  None of the ~100+ call sites needed to change, only `fonts.rs` itself and
  the bundled `.ttf`s. The 600 (SemiBold) weight has no named instance in
  Google Fonts' variable source (JetBrains Mono's own static family skips it
  too), so it was instanced without `--update-name-table` and had its name
  table set by hand afterward — see `assets/fonts/LICENSE-OFL.txt`.
- **Spacing/sizing — the visible "more spacious" delta**: sidebar width
  248→272px (the mockup's own default); `Density::Compact`'s title/tab-bar
  height 32→30px (now matches Maho's `--control-h-sm` exactly — `Comfortable`
  (38) and `Spacious` (46) already matched `--control-h-md`/`--control-h-lg`
  precisely, nothing to change there); button/input/badge corner radius
  2→3px (Maho's `--radius-sm`) and modal-level containers (command palette,
  settings panel) 4→10px (`--radius-lg`); UI font sizes bumped along Maho's
  scale (10→11 micro, 12→13 body-sm, 14→15 body-md, 16→17 body-lg, 20→22
  heading); off-scale `.spacing()` gaps (6/7/10/3/20px) rounded up to the
  nearest step on Maho's `--space-*` scale (8/12/4/24px) — gaps already
  exactly on that scale (2/4/8/12/16px, which is most of them) were left
  alone. `devscribe_core::theme`'s `FONT_SIZES`/`SPACING`/`GEOMETRY`
  constants were updated to the same numbers for documentation parity, but
  — as before this change — nothing in the UI layer actually reads them;
  every call site still owns its own literal.

**Post-Maho follow-up, from direct visual feedback comparing the built app
against the mockup/screenshots side by side**: several chrome elements read
as too small even though their sizes matched the mockup's own CSS
literally — the title bar's "Run anything"/Assist buttons, the sidebar's
EXPLORER header + its icon buttons, the git branch row, the CHANGES section
(header, clean-tree row, per-file rows), and the project-switcher's
name/path. Bumped these in two further rounds beyond the initial Maho port,
then did one more app-wide pass over every remaining `text_scale::px(N)`
call site (tab bar, status bar, command palette, settings panel, welcome
screen, diff/JSON/search views, toast, flash, find bar, context menu, and
the shared badge/micro widgets in `widgets.rs`) — micro text moved from
10/11px up to 13px, body-ish text from 12/13px up to 15px, and the
title-bar wordmark/logo got an explicit bump too (15→18px / 18→22px). None
of this is mockup-literal anymore; it's tuned by eye against the running
app, which is expected to keep moving as more direct feedback comes in.

**2026-08-26 update — collapsible sidebar**: the Claude Design mockup added
a `sidebarCollapsed`/`sidebarOpen` toggle — a new "Collapse sidebar" button
(`panel-left-close`) in the sidebar footer, and, when collapsed, a narrow
34px icon rail (project glyph, a spacer, Settings, an "expand" button)
replacing the full sidebar entirely. Ported as `State.sidebar_collapsed`
(new `Message::CollapseSidebar`/`ExpandSidebar`, `sidebar.rs::view`
branching to a new `collapsed_rail()` when set) — icon glyphs are plain
text (`«`/`»`), matching this codebase's established "no SVG icons"
convention rather than the mockup's own `MahoDesignSystem_2eb4f3.Icon`
component. `collapseSidebar`'s mockup handler also closes the projects
dropdown and clears any pending menu state; ported the same way (closes
`state.projects_open` and `state.ctx_menu`, both of which would otherwise be
anchored to sidebar content that's about to disappear). New tests:
`state::tests::collapse_sidebar_sets_collapsed_and_closes_menus_anchored_to_it`,
`expand_sidebar_clears_collapsed_without_touching_other_state`.

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

### Phase 6 — Command palette & working-tree-diff shortcut — ✅ done

- **`ViewWorkingTreeDiff`**: a new `Message`/`PaletteAction` (global `⇧⌘D`,
  plus a "Diff: open working tree changes" palette entry) distinct from the
  existing `ViewDiffOfActiveFile`. Rather than just aliasing the existing
  active-file diff action — which would no-op with nothing open — it diffs
  the active file if one's open, else falls back to the first entry in
  `state.changed_files`, so the command actually does something ("open
  working tree changes") even with zero tabs open, as long as *something* in
  the project has changed. Still strictly per-file under the hood (`TabKey::Diff(PathBuf)`,
  unchanged from Phase 1) — there's no aggregate multi-file diff view, and
  building one would be a much larger feature than this phase's scope; this
  just picks *which* file's diff to jump to. The palette entry itself is
  gated on there being something to actually diff (active file or a
  non-empty `changed_files`), same "no fake controls" call as the existing
  `ViewDiffOfActiveFile` entry's active-file gate.
- **Contextual "Diff:" grouping**: `command_palette.rs` now splits results
  into commands + a "FILES" section whenever the query contains "diff" —
  matching the mockup's illustrated layout, which is itself static (the
  mockup doesn't actually filter by query; the grouping had to be built for
  real). Implemented as a pure rendering-layer partition of the *same*
  `filtered_palette_entries` list `entries[i]`/`selected` already drive —
  entries with an `"Open: "`-prefixed label go under "FILES", everything
  else stays above it — so keyboard nav and Enter-to-run needed no changes
  at all, only which visual group a row's index lands in. No special-casing
  needed to make the new `ViewWorkingTreeDiff` entry itself show up under
  "diff": its label already contains the word, so the existing substring
  filter in `filtered_palette_entries` surfaces it for free. The mockup's
  other two "Diff:" rows ("compare with branch…", "stage this hunk") are
  decorative in the mockup itself (no real handler backs them there
  either), so — consistent with every other "no fake controls" call in this
  doc — they're not built.
- New tests in `state.rs`: `view_working_tree_diff_prefers_the_active_file`,
  `view_working_tree_diff_falls_back_to_first_changed_file_with_no_active_tab`,
  `view_working_tree_diff_is_a_noop_with_nothing_to_diff`,
  `diff_working_tree_palette_entry_is_gated_on_having_something_to_diff` —
  the fallback-ordering tests explicitly reset `changed_files` to empty
  before seeding their own fixture data rather than trusting
  `State::default()`'s real-repo introspection, since these tests run
  inside DevScribe's own (often locally-modified) working tree and ambient
  changed files would otherwise make which file "wins" the fallback
  nondeterministic.

### Phase 7 — Settings panel v2 — ✅ done

- **Category rename/drop**: `SettingsCategory` is now `Explorer`/`Editor`/
  `Toolchains`/`Shortcuts`/`About` (renamed from `Keymap`, `Advanced`
  dropped entirely) — and, new as of this phase, *every* category has real
  content. `placeholder_content` (the Phase 4-era "settings aren't available
  yet" fallback) is gone from `settings_panel.rs` along with the wildcard
  match arm that fed it.
- **Product decision, asked rather than assumed**: keep DevScribe's
  font-size stepper in Editor even though the mockup's refresh dropped it —
  confirmed with the user before touching `editor_content`, since dropping
  it would leave editor font size with no control that sets it.
- **Toolchains — real, not the mockup's four-server fantasy**: the mockup
  shows `rust-analyzer`/`clangd`/`jdtls`/`typescript-language-server` all
  reporting READY/FETCHING. DevScribe only ever speaks to one language
  server, so `toolchains_content` shows exactly that one, real
  (`state.lsp_status`) — fabricating status rows for three servers with zero
  backing would be the same class of thing Phase 4 already declined to do
  for "Install toolchains automatically". `LspStatus` gained a `describe(p)`
  method (color + label) so `status_bar.rs` and the new Toolchains/About
  content render from one source of truth instead of two copies of the same
  `match` drifting apart.
- **Shortcuts — every currently-real keybinding, not the mockup's literal
  10 rows**: the mockup's list includes "New window"/"Open folder"/"Save
  as", none of which have a working handler in DevScribe (no multi-window
  support, no folder-picker dependency yet — Phase 9 — no save-as flow), so
  documenting them as shortcuts would be false. Conversely, the table
  includes several real shortcuts the mockup's illustrative list didn't
  happen to mention (Close tab, Close other tabs, Reopen closed tab, Reveal
  in tree, Escape) — pulled directly from `state::global_keys` rather than
  transcribed from the mockup. Landing this table meant two shortcuts it
  wanted to list didn't actually exist yet, so they got wired for real
  rather than documented as aspirational: global `⌘I` (`Toggle assist`,
  previously title-bar-button-only) and `⌘/` (`Message::OpenShortcutsHelp`,
  opens Settings straight to Shortcuts in one step).
- **About — a real BUILD/PLATFORM/TOOLCHAIN table, not the mockup's
  BUILD/RUNTIME/TOOLCHAINS/LICENSE**: `RUNTIME` (the compiling rustc's
  version) and `LICENSE` are dropped rather than invented — neither is
  tracked anywhere in this workspace (no `build.rs` capturing `rustc
  --version`, no `license` field in `Cargo.toml`). What *is* shown is real:
  crate version (`env!("CARGO_PKG_VERSION")`) and platform
  (`std::env::consts::OS`/`ARCH`), both compile-time-free. Also fixed
  `title_bar.rs`'s hardcoded `"V1.4.0"` (a leftover from the mockup's own
  fictional versioning) to the same real version string — leaving it as a
  *different, fake* number right next to the new real one in About would
  have been a fresh inconsistency introduced by this very phase.
- **`Message::ViewWorkingTreeDiff`/Phase 6 stays untouched** — Shortcuts
  just documents its `⇧⌘D` binding alongside everything else; no changes to
  its behavior.
- **"Save on focus loss"**: new, for real — `iced::window::events()` joins
  `state::subscription`, mapped to `Message::WindowUnfocused` on
  `Event::Unfocused` (everything else, including every other window event,
  is a no-op). Handler saves every dirty open file, not just the active one
  — closer to what "save on focus loss" implies than a single-file version
  would be. Collapses successes into one "Saved N files" toast instead of
  stacking one per file; failures (permission errors, a file deleted from
  under an open tab) still get their own. Defaults **off** — unlike this
  project's other toggles, flipping this one silently starts writing to
  disk, so it shouldn't be on by default without the user choosing that.
- New tests in `state.rs`: `save_all_dirty_files_saves_only_dirty_files_with_one_summary_toast`,
  `window_unfocused_only_saves_when_the_toggle_is_on`.
- **Post-landing fix**: clicking anywhere in the new Toolchains/Shortcuts/
  About content (a status row, a shortcut's key label, blank space between
  sections) closed the whole settings modal instead of doing nothing. Root
  cause predates this phase — `settings_panel.rs`'s `panel` was never
  shielded from the backdrop `mouse_area` beneath it in the `stack!`, so any
  click that isn't captured by a real button/input along the way falls
  through and hits the backdrop's `on_press(CloseSettings)`. Explorer/Editor
  never surfaced this because their content is almost entirely buttons
  (theme grid, density row, toggle rows, steppers) with little dead space to
  click; `status_row`/`shortcut_row` are plain `container`/`row` with no
  such capture, so Phase 7 made the existing gap easy to hit. Fixed by
  wrapping `panel` in a `mouse_area(...).on_press(Message::Noop)` — it
  captures whatever inner buttons don't, without changing their own
  behavior (a button still claims its own press before the wrapping
  `mouse_area` gets a look). Found and fixed the identical latent bug in
  `command_palette.rs` (the footer hints and "No matches" text have the same
  non-capturing shape) proactively, same fix. Left `context_menu.rs` as is —
  its one non-button row falling through just dismisses the menu, which is
  harmless there (dismiss is already the menu's own backdrop behavior),
  unlike the settings panel and palette where staying open is the whole
  point.

### Phase 8 — Sidebar git-header states — ✅ done

- **`Repo::ahead_behind()`**: new in `devscribe-core`, the standard
  merge-base approach — `gix`'s `Repository::merge_base(local, upstream)`
  splits history into "ahead" and "behind", each counted via
  `<tip>.ancestors().with_hidden([base]).all()` (the traversal equivalent of
  `git rev-list <tip> ^<base>`). Finding "upstream" itself needed
  `Repository::branch_remote_tracking_ref_name` off the current branch's
  full ref name (from `head_name()`, already used by `branch_name()`) — the
  same concept `git`/`git2` call "upstream", just resolved through `gix`'s
  config-reading rather than a single convenience call, since none of `gix`
  0.86's public API packages the whole "ahead/behind vs. upstream" operation
  as one function. Returns `None` — not an error — when there's no upstream
  to compare against (no remote, branch isn't tracking one, detached
  `HEAD`), same "expected, common state" treatment `branch_name()` and
  `head_text()` already give their own None-cases.
- Wired into `state.rs` as `State.ahead_behind: Option<(usize, usize)>`,
  refreshed in lockstep with `state.changed_files` inside
  `refresh_changed_files` (startup + after every `EditorSave` — same
  staleness story that function already documents, now covering both git
  reads it triggers instead of one).
- **Sidebar project-switcher header**, previously just a project name/path
  button with nothing below it: a new git-status row directly under it —
  branch name (+ `▲a ▼b` only when `ahead_behind` is `Some`, never a
  fabricated `▲0 ▼0` for a repo with no upstream) when `state.repo` is
  `Some`, else "No repository" (the "not a git repo" text previously only
  existed down in the footer and status bar, never up here where the mockup
  puts it).
- **"Working tree clean"** row (green dot + label) fills the gap the CHANGES
  section leaves behind for a git repo with zero changes — previously a
  clean tree just meant nothing rendered there at all, giving no visual
  confirmation the absence was deliberate rather than, say, changes failing
  to load.
- New tests in `devscribe-core/src/git.rs`:
  `ahead_behind_counts_diverged_commits_against_the_tracked_upstream` (a
  real two-repo fixture — `git init` + `git clone` + a commit on each side
  after the clone, so `local` ends up genuinely diverged: one commit ahead,
  one behind — asserts `Some((1, 1))`) and
  `ahead_behind_is_none_without_an_upstream`.

### Phase 9 — Multi-project welcome screen & untitled buffers — ✅ done

Both real Phase 9 items are done — the multi-project welcome screen, and
(as a follow-up pass) unsaved/"untitled" buffers + Save As. The one
remaining bullet below (window control buttons / logo) was never scoped
work to begin with, just a standing, deliberate exclusion.

- **Product decisions, made explicitly before building anything**: startup
  auto-reopens the most recently used project (VSCode-style) rather than
  always stopping at the welcome screen or always reopening CWD — the
  welcome screen now only shows on first run or after "Close project".
  "New project" reuses the same folder picker as "Open folder" and
  `git init`s the result if it wasn't already a repo. "Clone repository"
  started as a disabled "coming soon" toast (real `git clone` —
  network/auth/progress — was always out of scope for this pass) and was
  later removed from the welcome screen entirely rather than left as a
  dead button.
- **New dependencies**: `rfd` (native async folder picker —
  `AsyncFileDialog::pick_folder()`), `dirs` (per-OS config directory), and
  `serde` promoted from transitive (via `serde_json`) to direct, with the
  `derive` feature, so the persisted recent-projects list can
  `#[derive(Serialize, Deserialize)]` instead of hand-building JSON.
- **`devscribe/src/recent_projects.rs`** (new): the persisted
  `~/.config/devscribe/recent_projects.json` — `path` + `last_opened_ms`
  only. Branch name, change count, and language glyph are deliberately
  *not* persisted (they'd go stale between launches); `state.rs`'s
  `compute_welcome_rows` recomputes them live, transiently, from
  `recent_projects` whenever it changes. `load`/`save` are best-effort
  (empty list / silently-swallowed write failure, never a hard error) —
  this list is a convenience, not state the app depends on to function.
- **`State`**: new `welcome_open: bool` (drives a genuine full-window
  replacement in `shell::view` — not another stacked overlay like
  `settings_panel`/`command_palette`, since the OS still owns window
  decorations independently of `title_bar.rs`, confirmed safe to omit
  entirely while this is showing), `recent_projects`, `welcome_rows`
  (the live per-row display data), and `loading_project` (drives the
  "Loading workspace" overlay while a background load is in flight). A new
  `ProjectSnapshot`/`snapshot_project` pulls the tree-walk + git-summary
  logic that used to live only inline in `Default::default()` into a
  function shared with the new background loader below.
- **Background project loading, same shape as the search bug fix's
  lesson**: `start_loading_project` spawns an `iced_runtime::task::blocking`
  thread (the same `catch_unwind` + `crate::logging::error` defensive shape
  as `start_search`) rather than walking the tree/running git status on the
  UI thread — exactly the mistake the project-search crash saga spent nine
  passes tracking down, not one worth repeating here. `Repo` itself never
  crosses the thread boundary (it isn't `Clone`, and `Message` is `Clone`
  project-wide) — the background thread returns plain data
  (`ProjectSnapshot`), and `Repo::open` (cheap — opens refs/HEAD, not a
  status walk) gets called again synchronously wherever a snapshot is
  applied to `State`.
- **LSP restart on project switch**: `Subscription::run(lsp_worker)` (a
  zero-arg `fn` pointer, hard-coded to `current_dir()`) became
  `Subscription::run_with(state.root.clone(), lsp_worker)` — `run_with`'s
  data is part of the subscription's identity, so a project switch
  changing `state.root` makes the runtime tear down the old LSP worker and
  spawn a fresh one for the new root automatically, no manual kill/respawn
  bookkeeping needed. `lsp_worker`'s `-> impl Stream + use<>` return type
  was required to opt back out of edition-2024's default RPIT lifetime
  capture, which otherwise made it incompatible with `run_with`'s
  `fn(&D) -> S` bound. The whole subscription is skipped while
  `welcome_open` — there's no root to spawn one for.
- **No confirm-before-discard prompt** on switching/closing a project: the
  app already has no dirty-file guard anywhere else (`CloseActiveTab`
  already discards silently), so this matches existing precedent rather
  than introducing a new one.
- **`devscribe/src/ui/welcome.rs`** (new): left rail — with the real
  title-bar fold mark (`LogoMark`, a size up), not a generic lang-badge
  placeholder — recent-projects list, and Open folder / New project
  actions ("Clone repository" was removed after initially shipping as a
  disabled placeholder), plus a "Loading workspace" overlay. Icons are
  plain text glyphs, not SVG — matching every other icon in the app
  (chevrons, badges), so this doesn't introduce SVG rendering for the
  first time just for this screen.
  **`sidebar.rs`**: the project-switcher header's `ToggleProjects` toggle,
  previously wired to nothing, now opens a real dropdown (`projects_menu`,
  same backdrop + positioned-`container` pattern as
  `tab_bar::overflow_menu`) — current project pinned first, other recents,
  "Open folder…", "Close project".
- **Test hermeticity note**: `State::default()`'s auto-reopen reads (and,
  on success, writes) the *real* `~/.config/devscribe/recent_projects.json`
  — fine for the actual app, but every one of the dozens of existing tests
  that construct `State::default()` would otherwise depend on, and mutate,
  this machine's real config file. `state.rs`'s `startup()` is split into
  `#[cfg(not(test))]` (the real load/auto-reopen/persist) and `#[cfg(test)]`
  (always a deterministic, disk-free "no project open" seed) — the test
  build never touches the real file at all.

**Unsaved/"untitled" file buffers + real Save As** — the last real Phase 9
item, done as a follow-up pass:

- **The gap, and why it wasn't a small change**: `EditorState.path: PathBuf`
  (not `Option`) is the tab-identity key used everywhere —
  `TabKey::File`/`OpenTab::key()`, `find_editor`/`find_editor_mut` (24
  call sites combined), `active_file_path` (21 call sites), LSP notify,
  diff, save, and the tab-bar label all assume a real path.
  `devscribe_core::Document`, underneath, already had none of that problem
  — `path: Option<PathBuf>`, a non-panicking `save()` that returns a
  `NotFound` error when it's `None`, and `set_path()` (already added for
  Phase 5's rename) — so the gap was entirely in the app crate, one layer
  up from where it could've just been reused.
- **The design that avoided a ~45-call-site rewrite**: instead of making
  `EditorState.path` optional too, an untitled buffer gets a **synthetic
  but real `PathBuf`** as its identity — `Untitled-1`, `Untitled-2`, ...
  (`State.next_untitled_id`, monotonic, VSCode-style naming), a bare name
  with no directory component so it can never collide with a real project
  file (those are always absolute, from `fs_tree::walk(&state.root)`).
  "Untitled-ness" is a property of `editor.document.path().is_none()`,
  checked only where it actually matters (saving) — every other
  path-keyed mechanism keeps working completely unchanged, and several
  (LSP's extension-gated `is_lsp_language`, diff's "no `head_text` ⇒
  Untracked") already degrade gracefully for a no-extension path, for
  free.
- **`begin_untitled_buffer`**: mirrors `open_or_focus_file`'s shape from
  `Document::empty()` instead of `Document::open`. Reachable only from the
  command palette ("New untitled file") — `⌘N`/`⇧⌘N` were already taken
  (sidebar-draft new-file/new-folder, a deliberately different thing per
  the note above), and the palette had zero file-creation actions to
  conflict with.
- **Saving it**: `save_current_file` now checks
  `editor.document.path().is_none()` first and hands off to
  `save_file_as` (a `Task::perform`-wrapped `rfd::AsyncFileDialog::save_file()`
  call — the save-file counterpart to the welcome screen's already-added
  `pick_folder()`, same shape) instead of calling `.save()` straight into
  the existing "document has no path" error. `save_all_dirty_files` (the
  save-on-focus-loss path) deliberately skips untitled+dirty buffers
  rather than popping a blocking native dialog the moment the window loses
  focus — those still save normally whenever the user explicitly asks.
- **`complete_save_as` reuses `rename_open_tab`** (built for Phase 5's file
  rename) rather than reinventing it: repointing a tab's identity from an
  old path to a new one in place — no close/reopen, so no lost
  cursor/undo/find state, plus LSP old-close/new-open and `active_tab`/any
  matching `Diff` tab key fixup — is exactly the same bookkeeping "turn a
  synthetic-path tab into a real-path tab" needs. The one thing reused
  as-is *doesn't* do: `EditorState::new` only derives `language`/
  `highlights`/JSON-parsing once, at construction — a pre-existing,
  already-accepted limitation for an ordinary rename (rare to change
  extension), but core to this flow (blank buffer with no highlighting →
  a real language *is* the point), so `complete_save_as` explicitly
  recomputes those after the repoint, then does the actual disk write
  (unlike a real `fs::rename`, nothing existed at the new path before).
- **Pre-existing latent bug, fixed alongside this**: `⌘N`/`⇧⌘N`
  (`Message::BeginDraft`) weren't gated on a project being open — on the
  welcome screen (`state.root` is `PathBuf::new()` there), pressing them
  silently started an invisible draft that would've written into the
  process's CWD if ever committed. Now a no-op while `welcome_open`, same
  as the LSP subscription's existing gating.
- Tab-bar styling: an untitled tab gets the mockup's accent-tinted `TXT`
  glyph (`tab_bar.rs`'s `file_tab_label`) instead of a language badge —
  cosmetic only, the label/dirty-dot logic already worked unchanged.

**Still deferred, unchanged**:

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

### Bug fix — project search freezing/killing the app while typing

Reported symptom: typing into the project-wide search box could take down
the whole app process — no error dialog, just gone, consistent with an
allocator abort or an OS OOM-kill rather than a catchable panic (which
would have unwound with a message). Landed in two passes as the picture
sharpened:

**First pass (necessary, turned out not sufficient on its own)**: audited
`devscribe_core::search::search_text` and found it had no cap of its own —
callers' result limits (`MAX_SEARCH_RESULTS` in the project-wide panel,
Phase 3.5's find-in-file) were only ever applied to the *finished* `Vec`,
after every matching line had already been fully scanned and — since every
`SearchHit` clones its whole line into `preview` — cloned. For one large
file with many matches on long lines (a lockfile, a minified bundle, a
log), that's a multi-KB line matched thousands of times multiplying out to
gigabytes. Fixed: `search_text` gained a `max_hits` parameter and now stops
scanning — not just truncates — the moment it's reached;
`recompute_search` passes *remaining* budget per file so one file can't
spend the whole project's budget by itself; a new `MAX_SEARCH_FILE_BYTES`
(2 MiB) skips oversized files before ever reading them. Confirmed real
with `search::tests::stops_scanning_as_soon_as_max_hits_is_reached` and
`state::tests::recompute_search_caps_a_single_file_with_many_matches_on_one_line`.

**Second pass, from the user's follow-up repro (typing a single letter —
"g" alone, before finishing "git" — was enough to hang)**: the caps bound
*memory per file*, but the actual freeze was `recompute_search` itself —
walking and reading every matching file in the project, synchronously, on
the UI thread, *on every keystroke*, with no cancellation of the previous
keystroke's now-obsolete work. A broad single-letter query touching
hundreds of files is real, uncancellable CPU/IO work regardless of any
per-file cap. Compared against how established tools avoid this: VSCode
runs search in a background process (ripgrep), debounced and cancellable,
so the UI thread is never blocked; Notepad++ doesn't search live at
all — "Find in Files" only runs on explicit submit. DevScribe has no
background-worker infrastructure for this (same synchronous, "start naive"
architecture as the rest of the app), so the Notepad++ shape was the
correct-sized fix, and it also happens to match what the panel's own
empty-state copy already promised ("Type a query and press Enter") — a
real mismatch between that copy and what the code did, not just a perf
tweak. `Message::SearchQueryChanged` now only updates the text;
`recompute_search` runs solely from `Message::SearchSubmit` (Enter). New
`state.search_last_query` tracks what was actually last searched (distinct
from the live-typed `search_query`), so `search_view.rs` can tell an
un-submitted edit apart from real results and show "Press Enter to search
for…" instead of stale results or a misleading "No matches" for a query
that was never run. Covered by
`state::tests::search_only_runs_on_submit_not_on_every_keystroke`.

**Third pass, from a second follow-up repro (typed "git", pressed Enter,
still hung)**: submit-only stopped every-keystroke re-runs, but a single
deliberate submit could still be doing far more work than it needed to.
`recompute_search` was running the *full syntax highlighter over every
whole file* that had at least one match (`line_segments`, feeding
per-token colors into the results preview) — for a broad, common query
like "git" (matches "digit", "legit", `.gitignore`, any comment mentioning
git…) across a real-sized project, that's potentially hundreds of
full-file tokenization passes, synchronously, for one submit. This was
strictly worse than the memory-focused first pass suggested: capping
*match count* (pass one) bounds how many `SearchHit`s get collected, but
highlighting ran per matching *file*, before any hit-level cap applied —
a file that matched once still got fully tokenized. Fixed by removing
per-file syntax highlighting from search results entirely: `SearchResult`
no longer carries `segments`, `line_segments` is deleted, and
`search_view.rs`'s `match_line_runs` now only splits the plain preview
line into up to three runs (before/matched/after) rather than resolving
per-token colors — the matched substring is still highlighted (accent
background), just not the surrounding code. Also added
`MAX_SEARCH_FILES_SCANNED` as a hard ceiling on how many files a single
search reads and scans at all, regardless of match count — a query that's
rare or absent across a very large project previously had no bound on how
many files it would walk, stat, and read before giving up. Real numbers,
not just theory: search for "git" against DevScribe's own `devscribe/`
crate (35 matches) dropped to ~6ms end to end once highlighting was
removed.

**Fourth pass**: lowered `MAX_SEARCH_FILES_SCANNED` from an initial 20,000
to 3,000, on request. Even with per-file highlighting gone, reading and
scanning tens of thousands of files synchronously on the UI thread for one
submit is still real, noticeable latency on a large enough project — 3,000
stays generous for a typical project while keeping a worst-case search
feel closer to instant than merely "eventually bounded." Covered by
`state::tests::recompute_search_stops_after_max_files_scanned` (a single
matching file placed, by construction, just past the cap in walk order —
proves files beyond the limit are never even read, not just excluded from
the final results).

**Fifth pass, from a third follow-up ("still crashes")**: this time asked
two decisive questions before changing anything else, rather than guessing
again — (1) does the window actually disappear/process exit, or does it
stay open but stop responding? (2) run from a terminal: does anything
print? Answers: stays open, unresponsive; nothing prints, even from a
terminal. That rules out an actual panic (would print) and a real
crash/OOM-kill (window would be gone) — it's a genuine hang: the update
thread stuck inside a blocking call, never returning to let `iced`
repaint or process input. The prime suspect: `std::fs::read_to_string`
blocking far longer than a local disk ever would, or forever, which
happens for reasons that have nothing to do with file *size* or *count*
(so the four passes above, all about bounding those, couldn't have fixed
it) — a project directory on a network mount, or a cloud-sync client
(OneDrive/Dropbox/Google Drive "files on demand") where opening a
placeholder file triggers a synchronous download.

Rather than add another guessed-at cap, moved project search off the UI
thread entirely and made it debounced + cancellable — the same shape
VSCode's search-as-you-type uses (there: a debounced, cancellable ripgrep
subprocess; here: a debounced, cancellable background thread, since
DevScribe has no subprocess-based search and isn't about to grow one for
this). Concretely:

- `Message::SearchQueryChanged` now only records *when* the box was last
  edited (`State::search_query_changed_at`, private). A new recurring
  `Message::SearchDebounceTick` (every 100ms — coarse on purpose, it only
  needs to notice the delay elapsed promptly, not precisely) checks
  whether `SEARCH_DEBOUNCE_DELAY` (300ms) has passed since; if so, it
  starts the search. `Message::SearchSubmit` (Enter) still exists and
  bypasses the wait entirely, for anyone who doesn't want to wait it out.
- The actual file-reading loop was extracted into a pure `run_search(files,
  query) -> SearchOutcome`, decoupled from `State` so it can cross a
  thread boundary. `start_search` calls it via
  `iced_runtime::task::blocking` (a new direct dependency — `iced`'s own
  public API re-exports `Task`/`Handle` but not this free function) — runs
  on a plain `std::thread::spawn`, not async/tokio, precisely so a stalled
  syscall only ever blocks that one thread. Its result comes back as
  `Message::SearchCompleted(SearchOutcome)`, applied only if
  `outcome.query` still matches `state.search_query` — the guard against a
  slow search's result landing after the user has moved on to a different
  query.
- Cancellation: `start_search` first aborts whatever `Handle` is already
  in `state.search_task_handle` (from `Task::abortable()`) before starting
  a new one. This is best-effort by necessity, documented as such on
  `start_search` itself: `std` has no safe way to kill a thread mid-syscall,
  so "cancel" only ever means "stop listening for this one's result" — an
  abandoned search's thread keeps running (or stays stalled) in the
  background, harmlessly, rather than blocking anything.
- `search_view.rs` gained a "Searching…" state (`state.search_in_progress`)
  alongside the existing "press Enter" (typed, debounce hasn't fired yet)
  and "No matches" (searched, found nothing) states — three genuinely
  different situations that all look identical without this.

New tests, all exercising `update()`/`run_search` directly rather than
needing a real background thread: `search_query_changed_does_not_start_a_search`,
`search_debounce_tick_waits_for_the_delay_to_elapse`,
`search_debounce_tick_starts_a_search_once_the_delay_elapses`,
`search_submit_bypasses_the_debounce_and_starts_immediately`,
`search_completed_applies_results_only_for_the_still_current_query`,
`starting_a_new_search_aborts_the_previous_ones_handle` (checks
`Handle::is_aborted()` directly).

**Sixth pass — the app still hung even backgrounded, so added real
logging instead of another guess.** New `devscribe/src/logging.rs`,
initialized first thing in `main`: truncates `<project root>/.devscribe.log`
each run (path fixed at *compile* time via `CARGO_MANIFEST_DIR`, not the
runtime working directory — the `.desktop` launcher this project set up
earlier has no `Path=` key, so a click-to-launch's `cwd` defaults to
`$HOME`, and a `current_dir()`-based log path would've silently landed
there instead every time it launched that way; gitignored, since it now
lives inside the repo) and installs a process-wide panic hook (fires for
*any* thread, including the background search thread — `set_hook` isn't
per-thread) that logs every panic there in addition to the default stderr
output, so a
desktop-icon launch (no attached terminal — the case here, per an earlier
`.desktop` file this project set up) stops being a black box. `run_search`
logs the query and file count on entry, one line *before* each file read
(the read itself is the one unbounded, no-timeout step — if the app hangs
again, the log's last line names the exact file it was reading), and a
summary on completion; `start_search` logs each spawn/abort;
`SearchCompleted` logs whether a result was applied or discarded as stale.
`start_search`'s background closure also wraps `run_search` in
`std::panic::catch_unwind` — redundant with the global hook for *knowing a
panic happened*, but names *which operation* it was, which a hook firing
from an unnamed background thread alone wouldn't. New direct dependency:
none beyond what search already added — logging is plain `std`
(`OnceLock`, `Mutex<File>`), no crate needed. Also rebuilt the release
binary the `.desktop` launcher (an earlier request in this project) points
at, since debug-only rebuilds wouldn't reach that launch path.

**Seventh pass — the logging from pass six paid off immediately.** Full
log from the next repro (searching "git" in this repo):
`run_search: finished "git" — 200 results from 56 files read in
89.959262ms` immediately followed by `SearchCompleted: applying "git" (200
results)`, then nothing further. Decisive: search itself is fast and
completes cleanly — the earlier five passes' fixes hold up. Whatever
happened, happened *after*, with 200 results now sitting rendered on
screen. That pointed straight at a regression pass five itself introduced:
`subscription()`'s `SearchDebounceTick` timer (`iced::time::every(100ms)`)
was unconditional — part of the fixed subscription batch regardless of
`state`, rather than gated on there being an actual pending debounce to
check. A message on *any* `iced::Subscription` tick triggers a full
`view()` rebuild, so this was a permanent 10-times-a-second full-app
redraw for the entire remaining lifetime of the process, not just during
the brief window a debounce is actually pending — and with ~200 result
rows now part of that tree, ten full rebuild-layout-paint cycles a second,
forever, is real sustained load capable of looking exactly like "the app
hangs" without anything actually being stuck (consistent with every
"crashed" report in this saga describing a live-but-unresponsive window,
never a real exit).

Fixed by making `state::subscription` take its `state` parameter for real
(it was `_state`, ignored, since Phase 1) and only including the debounce
tick `Subscription` when `state.search_query_changed_at.is_some()` — which
goes back to `None` the instant a search actually starts
(`SearchDebounceTick`'s "due" branch, and `SearchSubmit`), so the tick now
only exists for the ~300ms window between an edit and the search it
triggers, never longer, regardless of how many results end up on screen
afterward.

**Eighth pass — same log signature after the seventh-pass fix, which
narrowed things further.** Next repro's log ended at the identical point:
`run_search` finishes in ~93ms, `SearchCompleted: applying "git" (200
results)` logged, then silence — even with the unconditional-10Hz-tick
regression from pass seven already fixed. That rules the tick out as *the*
cause (it may still have been compounding it) and narrows this to: 200
results landing in `search_view.rs` and being handed to `iced` for a real
render is itself the trigger. A real blind spot in every test written so
far: `search_view::view()`'s unit tests (`state.rs`) only ever construct
the `Element` tree it returns — plain Rust struct allocation, cheap, and
exactly what passed. `cargo test` never opens a window, so none of them
touch actual layout or text shaping/painting, which is where a
widget-count-scaling cost would actually live. 200 results renders as up
to ~600 individual text widgets (`match_line_runs` splits each into up to
3 runs) — real work for a live renderer that a tree-construction test
structurally cannot see.

Two changes, addressing this from both ends since the exact mechanism
(construction vs. layout vs. paint) still isn't confirmed:

- `MAX_SEARCH_RESULTS` lowered from 200 to 50 (~150 text widgets at most
  instead of ~600) — a direct cut to whatever's scaling with result count,
  regardless of which stage turns out to be the actual cost.
- `search_view.rs` now logs immediately before building the results tree
  and immediately after, around `iced`'s hand-off point specifically. This
  is a real diagnostic boundary: application code can bracket tree
  *construction*, but not `iced`'s own layout/paint, which runs after
  `view()` returns. A hang whose last log line is "building" points at
  this function; a hang after "tree built, handing off to iced" points
  downstream, into the rendering pipeline itself, which no amount of
  application-level logging can see further into — the fix there would be
  fewer/lighter widgets (already underway) rather than different code.

**Ninth pass — found via real crash forensics, not another guess, and this
one's confirmed as the actual root cause.** Direct access to the machine
reproducing this turned out to be decisive: `journalctl`/`coredumpctl`
showed three real `SIGABRT` core dumps from `target/debug/devscribe`, all
today, zero from the release binary — pointing first at a debug-build-only
stack overflow (large unoptimized stack frames during layout of many
result rows; a null frame pointer at the abort site fit that pattern). But
the user then reproduced it with `--release` too, with no coredump at all
for that run — a genuine hang, not a fault signal, ruling out "debug-only"
as the full story. The log from that run ended at the same place as
every prior repro: `search_view: tree built, handing off to iced for
layout/paint`, then nothing. That, plus the release/debug split, pointed
at real, unavoidable rendering cost rather than a logic bug — so the next
step was finding out *what*, concretely, was that expensive.

`awk '{print length}' design/DevScribe.html | sort -rn | head` answered
it directly: **line 380 of this project's own design mockup is 1,401,229
characters long**, with 43 matches for "git" on it alone. Every one of
those 43 matches became a `SearchHit` whose `preview` was the *entire*
1.4-million-character line, cloned in full — then hitting one of them
even once was enough, handed to `iced` as a single text widget with over
a million characters in it. Not a framework bug, not GPU-driver flakiness,
not a stack-depth quirk of debug builds specifically (real cost, so slower
in debug and probably differently-shaped in release — a hang instead of
an overflow — but present in both): asking a text-shaping engine to lay
out a million-character run is genuinely catastrophic work, and it
explains every symptom observed across this whole saga — search
completing cleanly beforehand, the hang starting exactly at the `iced`
hand-off, and even the `kwin_wayland` compositor logging its own "main
thread was hanging temporarily" right around the same time.

Fixed at the actual source: `SearchHit` now separates the match's real
position (`col`, for cursor placement — unchanged, still indexes the true
line) from a bounded rendering snippet (`preview`, capped to
`PREVIEW_CONTEXT_CHARS` — 60 — of context on each side of the match, `…`-
marked wherever a side got cut) and that snippet's *own* local match
offset (`preview_col`, what `search_view.rs` now uses for highlighting,
replacing `col` there). Windowing happens in bytes first — nudging two
candidate cut points to the nearest valid UTF-8 boundary is O(1), not a
scan of the whole line — which already upper-bounds the resulting char
count, so there's no separate truncation pass needed after. `col` staying
exactly what it was means `SearchResultSelected`'s cursor placement
(`state.rs`) needed no change at all; only `search_view.rs`'s highlighting
call site switched from `hit.col` to `hit.preview_col`. `MAX_SEARCH_RESULTS`
restored to 200 (from the reactive drop to 50 two passes ago) — every
result's cost is now genuinely bounded regardless of the real line's
length, so there was never a good reason to show fewer once this landed.

New tests: `devscribe-core::search::tests::preview_is_capped_for_a_pathologically_long_line`
(a synthetic 2-million-character line, asserting `preview` stays under
1000 bytes while `preview_col` still correctly locates the match within
it, and `col` still holds the real, un-windowed position),
`preview_has_no_leading_ellipsis_when_the_match_is_near_the_start`, and
`state::tests::run_search_stays_fast_and_bounded_against_a_file_with_a_gigantic_line`
— the last one runs the *full* `run_search` path (not just `search_text`
in isolation) against a 1.4M-character line and asserts it completes in
under a second, directly exercising the exact failure mode rather than a
simplified stand-in for it.

**Confirmed fixed** by the user, in both debug and release builds. The
`devscribe/src/logging.rs` module added in the sixth pass stayed, scoped
down to what it's for going forward: `init()` still installs the
process-wide panic hook (any future panic, on any thread, still gets a
permanent record even from a desktop-icon launch with no attached
terminal), but every informational breadcrumb added purely to chase this
one bug — per-file "reading X", "spawning search", "applying"/"discarding
stale" completions, `search_view.rs`'s tree-construction markers — is
gone now that the actual root cause is known and fixed. `log` was renamed
to `error` to make that scope explicit at every call site: a healthy run
now leaves `.devscribe.log` empty, and anything written to it is worth
looking at.

**Known remaining gap, unrelated to the crash**: `search_text` splits
lines with `str::lines()` (`\n`/`\r\n` only), while `Document` (ropey,
`unicode_lines` on by default) also treats VT/FF/NEL/LS/PS/bare-CR as line
breaks. A file containing any of those makes search's line numbers
diverge from the document's from that point on. This used to also corrupt
syntax highlighting (now moot — there isn't any); what's left is
`SearchResultSelected` placing the cursor via `CursorPos { line, col }`
without validating those against the target document, which *could* land
on the wrong line for a file with such characters after a divergence
point. Rare in real source files; not fixed here since it's unrelated to
the crash and touches `devscribe-core::search`'s deliberately filesystem/
`Document`-independent design.

## Verification approach (per phase)

Same as every prior milestone in this project: `cargo build`, `cargo clippy
--workspace --all-targets`, `cargo test --workspace`, and a short smoke-run
of the binary after each phase. Visual/interactive claims (tab switching,
panel layout, hover states) can't be screenshotted in this sandbox — those
need a manual check by the user after each phase lands.
