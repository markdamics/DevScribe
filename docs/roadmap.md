# Roadmap

### 1. Implement Find and Replace (Roadmap Priority) - done
- The find_bar.rs UI skeleton exists but lacks replace functionality
- Why: This is a critical daily workflow feature; users will notice its absence immediately
- Scope: Extend devscribe-core/src/search.rs to support regex-based replacement with preview, and wire it through the UI state machine in state.rs

### 2. Complete Markdown File Support (Roadmap Priority) - done
- Markdown parser exists (tree-sitter-md) but isn't wired into the editor
- Why: Markdown is ubiquitous in projects; READMEs, docs, and changelogs need proper preview/editing
- Scope: Add a markdown preview panel (similar to json_view.rs) and ensure syntax highlighting works

### 3. Optimize AI Chat Memory Usage (Roadmap Note) - done 
- chat_panel.rs (40KB) is already substantial; the roadmap notes memory bloat
- Why: Running AI chat on a separate thread won't freeze the UI but memory footprint will degrade performance
- Scope: Implement message history pagination, streaming rendering, and consider offloading conversation context to disk (SQLite-backed sessions)

### 4. Keyword Settings Panel (Roadmap: Can't Edit) - done
- settings_panel.rs exists but keyword customization is hard-coded
- Why: Users need control over editor behavior (tab size, word wrap, line numbers density, etc.)
- Scope: Serialize keyword settings to JSON config file (already using serde), add UI form inputs, and reload on change

### 7. Breadth of Language Server Coverage
- server_install.rs auto-downloads clangd; add Pylance, rust-analyzer, ts-ls auto-install
- Why: Users expect their language of choice "just works"
- Scope: Extend the auto-install matrix; consider using the LSP registry or a manifest file

### 8. Refactor state.rs (256KB monolith)
- The entire state machine is in one file—difficult to maintain and test
- Why: This is a code health issue that will slow future feature development
- Scope: Split into state/editor.rs, state/sidebar.rs, state/chat.rs modules with clear message routing

### 9. Add Plugin / Extension System
- No extensibility mechanism yet
- Why: Community contributions will accelerate development; Rust WASM could be a future path
- Scope: Design a plugin manifest (JSON schema) and hook points (syntax highlighters, custom commands, keybindings)