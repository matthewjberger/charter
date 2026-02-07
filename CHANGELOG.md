## [0.1.3] - 2026-02-07

### 🚀 Features

- Add deep analysis capabilities
- Add comprehensive safety and Rust-specific analysis
- Add reverse call graph, semantic clustering, and data flow tracking
- Split into library and CLI binary for crates.io publishing
- Add python support and mcp server

### 🐛 Bug Fixes

- Preserve type names in compressed symbols, improve lookup
- Prevent cross-domain mega-clusters in function grouping
- Separate explicit panics from index operations in safety output
- Include Cargo.lock in version bump commits

### 🚜 Refactor

- Optimize tiers and compress symbols output

### 📚 Documentation

- Comprehensive README with all output files and examples
- Add badges to README
- Add emojis to title
- Improve README clarity and fix issues
- Remove known limitations section
- Add architecture documentation

### 🧪 Testing

- Add integration tests for charter functionality

### ⚙️ Miscellaneous Tasks

- Rename project from atlas to charter
- Prepare for crates.io release
- Add GitHub Actions workflow
- Bump version to v0.1.1
- Update changelog for v0.1.1
- Bump version to v0.1.2
- Update changelog for v0.1.2
- Bump version to v0.1.3
