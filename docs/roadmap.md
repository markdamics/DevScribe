# Roadmap

### 1. Tab Overflow UX Refinements - done
- Tabs already scroll horizontally and have an overflow ("⋯") dropdown (tab_bar.rs); still missing tab preview on hover and a quick tab switcher distinct from the command palette
- Why: Managing many open files is still cumbersome without a preview or a fast keyboard-driven switcher
- Scope: Add hover previews and a lightweight Ctrl+Tab-style switcher to tab_bar.rs

### 2. Keyboard Shortcuts Cheat Sheet
- A full keybinding reference already exists (Settings → Shortcuts, opened via ⌘/ or OpenShortcutsHelp); the command palette itself still doesn't show a shortcut next to each command
- Why: Discoverability is better than assumed, but palette entries still don't surface their own keybinding
- Scope: Add keyboard-hint labels to command_palette.rs's entries, sourced from the same table settings_panel.rs's Shortcuts category already lists

### 3. Split Editor Panes
- Enable side-by-side file editing (Cmd+\ or via command palette), optionally syncing scroll/navigation between splits
- Why: Comparing files or reference docs while editing is a common workflow that's currently impossible
- Scope: New split-pane layout in shell.rs's code_area, duplicating EditorCanvas per pane with independent (or linked) scroll state

### 4. Smart Problem Lens Improvements
- Quick Fix UI (a "💡" bulb/pop-up menu for available code actions), less invasive inline error message positioning, and per-severity hiding (hide warnings, keep errors visible)
- Why: Inline diagnostic annotations can clutter code today, with no way to act on or filter them
- Scope: editor_canvas.rs (problem lens rendering, ~lines 48-49), status_bar.rs

### 5. Search & Replace UX Refinements - done
- Preview mode showing all replacements highlighted before confirming, a clearer match-counter indication at list bounds ("1 of 47"), and a "?" regex quick-help tooltip
- Why: Find/replace itself is done (item 1), but its UX can still be refined
- Scope: find_bar.rs

### 6. Improved Completions Popup - done
- Documentation preview for the selected item (LSP `documentation` field), key hints ("Tab/Enter to insert, Esc to close"), signature help with active-parameter highlighting, and filter/sort by relevance, recency, or type
- Why: The popup currently shows bare labels with no context on what a completion actually does
- Scope: completions.rs

### 7. Chat Panel UX Enhancements - done
- Message threading (group user → AI response + tool calls visually), streaming "typing" feedback, copy/edit buttons per message, a context indicator (what files/selection a turn references), and persisted chat history with search/filter
- Why: chat_panel.rs is already substantial (1454 lines); item 3 addressed memory/performance, but these UX gaps remain
- Scope: chat_panel.rs

### 8. Visual Feedback for Async Operations - done
- Progress indicators for LSP indexing, git operations, and file watching, plus an operation-queue view of pending watchers/language-server init status
- Why: Currently it's unclear whether the app is frozen or just working
- Scope: status_bar.rs, LSP status plumbing in state/mod.rs

### 9. Status Bar Enhancements - done
- Clickable encoding indicator, clickable EOL (LF/CRLF) indicator, current language mode with click-to-switch, inline column/line position, and a git branch + dirty indicator (currently sidebar-only)
- Why: Useful per-file/per-repo context is scattered or missing from the one bar meant to summarize it
- Scope: status_bar.rs

### 10. Breadcrumb Trail Depth Indicator - done
- Clickable breadcrumbs that jump to that scope in the outline, plus hover context (function signature, class definition)
- Why: The breadcrumb trail is currently read-only, so it doesn't help navigation, only orientation
- Scope: breadcrumb_bar.rs

### 11. Theme/Accent Customization UI - done
- Custom accent color picker (currently fixed presets), live theme preview before applying, and a "High Contrast" toggle
- Why: Fixed presets don't cover every user's contrast/preference needs
- Scope: settings_panel.rs

### 12. Accessibility Pass - done
- Visible focus indicators across all panels, verified color contrast across themes, and ARIA-style/screen-reader hints
- Why: Currently there's no indication of what has keyboard focus in complex layouts
- Scope: Cross-cutting across theme.rs and ui/*.rs

### 13. Diff View Refinements
- Word-level diff highlighting (not just line-level), side-by-side diffs with synchronized scrolling, and an "ignore whitespace" toggle
- Why: Line-level-only diffs make small changes on long lines hard to spot
- Scope: diff_view.rs

### 14. Context Menu Enhancements
- Refactoring options (rename, extract function, etc.) and a "search for symbol across project" option
- Why: The right-click menu currently only covers file/tab operations, not code actions
- Scope: context_menu.rs