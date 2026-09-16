# Missing features

## **Performance & Rendering**

1. **Syntax highlighting caching** - Cache `tree-sitter` parse results more aggressively to reduce re-parsing on every view. The current system recomputes highlights frequently.

2. **Incremental viewport rendering** - Only render visible lines in `editor_canvas.rs` instead of all code. This would dramatically speed up large file navigation.

3. **Lazy diagnostic updates** - Debounce LSP diagnostic updates (currently they might trigger on every keystroke). Batch them for better performance.

## **Editor Experience**

4. **Smart paired bracket auto-close** - Auto-insert closing brackets/quotes with intelligent handling (skip if already present, indent correctly).

5. **Multi-cursor support** - Allow Cmd+D / Ctrl+D to select next occurrence and edit multiple positions simultaneously. This is lightweight and highly productive.

6. **Column selection** - Alt+click drag for column/block selection mode, useful for aligned edits.

7. **Bookmark/marker system** - Simple Ctrl+Shift+M toggle to mark important lines, navigable via quick menu (no persistence needed).

8. **Better undo/redo grouping** - Group rapid edits (typing) into single undo steps instead of per-character.

## **Navigation & Visibility**

9. **Breadcrumb improvements** - Make breadcrumbs clickable to jump to parent scope (already partially there but could be enhanced).

11. **Vertical scroll position indicator** - Show where you are in a long file more visually (current scrollbar is basic).

12. **Quick file peek** - Hover over imports/references in editor to see file preview without opening.

## **Code Intelligence**

13. **Inline type hints** - Show inferred types for variables (esp. Rust), configurable per language.

14. **Import auto-organization** - Detect unused imports and provide quick-fix, with sorting options.

15. **Bracket pair colorization** - Color matching bracket pairs differently to show nesting depth.

16. **Smart indentation** - Context-aware auto-indent based on language syntax (improve over simple indent matching).

## **Workflow**

17. **Recent files quick-access** - Cmd/Ctrl+E to fuzzy-search recently opened files (you have recent projects, but not per-session file history).

18. **Sticky scroll headers** - Keep function/class name visible at top when scrolling inside it (modern IDE feature, lightweight).

19. **Better selection shortcuts** - Extend selection word/line/block via Shift+movements.

20. **Configurable save on blur** - Option to auto-save when window loses focus (or on timer).

## **Tuning Suggestions**

21. **Font rendering quality** - Consider subpixel rendering options for text at various sizes (affects smoothness perception).

22. **Keystroke repeat tuning** - Optimize key-repeat sensitivity for cursor movement to feel snappier.

23. **Hover popup responsiveness** - Reduce hover delay from current ~500ms to 200-300ms for faster feedback.
