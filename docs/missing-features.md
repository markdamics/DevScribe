## Missing features

### 1. Keyboard Shortcuts Cheat Sheet
- A full keybinding reference already exists (Settings → Shortcuts, opened via ⌘/ or OpenShortcutsHelp); the command palette itself still doesn't show a shortcut next to each command
- Why: Discoverability is better than assumed, but palette entries still don't surface their own keybinding
- Scope: Add keyboard-hint labels to command_palette.rs's entries, sourced from the same table settings_panel.rs's Shortcuts category already lists

### 2. Split Editor Panes
- Enable side-by-side file editing (Cmd+\ or via command palette), optionally syncing scroll/navigation between splits
- Why: Comparing files or reference docs while editing is a common workflow that's currently impossible
- Scope: New split-pane layout in shell.rs's code_area, duplicating EditorCanvas per pane with independent (or linked) scroll state

### 3. Smart Problem Lens Improvements
- Quick Fix UI (a "💡" bulb/pop-up menu for available code actions), less invasive inline error message positioning, and per-severity hiding (hide warnings, keep errors visible)
- Why: Inline diagnostic annotations can clutter code today, with no way to act on or filter them
- Scope: editor_canvas.rs (problem lens rendering, ~lines 48-49), status_bar.rs

### 4. Diff View Refinements
- Word-level diff highlighting (not just line-level), side-by-side diffs with synchronized scrolling, and an "ignore whitespace" toggle
- Why: Line-level-only diffs make small changes on long lines hard to spot
- Scope: diff_view.rs

### 5. Context Menu Enhancements
- Refactoring options (rename, extract function, etc.) and a "search for symbol across project" option
- Why: The right-click menu currently only covers file/tab operations, not code actions
- Scope: context_menu.rs