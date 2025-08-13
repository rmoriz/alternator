# Agent Guidelines for Alternator

## Build & Test Commands
- **Build**: `cargo build` (dev), `cargo build --release` (production)
- **Test all**: `cargo test --verbose --all-features`
- **Test single**: `cargo test <test_name>` or `cargo test --test integration_tests`
- **Lint**: `cargo clippy --all-targets --all-features -- -D warnings`
- **Format**: `cargo fmt --all -- --check` (check), `cargo fmt` (fix)
- **Security audit**: `cargo audit`

## Code Style Guidelines
- **Imports**: Group std, external crates, then local modules with blank lines between
- **Error handling**: Use `thiserror::Error` for custom errors, implement `From` traits
- **Types**: Prefer explicit types, use `Option<T>` and `Result<T, E>` appropriately
- **Naming**: snake_case for functions/variables, PascalCase for types/enums
- **Async**: Use `tokio` runtime, prefer `async fn` over manual futures
- **Logging**: Use `tracing` crate with structured logging
- **Config**: Use `serde` for serialization, validate in constructors
- **Testing**: Place unit tests in same file with `#[cfg(test)]`, integration tests in `tests/`

## Project Structure
- `src/lib.rs`: Module declarations
- `src/error.rs`: Centralized error types with recovery strategies  
- `src/config.rs`: Configuration structs with validation
- Main modules: `mastodon`, `openrouter`, `media`, `language`, `balance`, `toot_handler`

## Git Workflow
- **MANDATORY**: Before ANY commit: ALL tests must pass, ALL lint must pass, ALL typecheck must pass. NO EXCEPTIONS.
- **CRITICAL**: Every change MUST be committed to git with a descriptive commit message
- **CRITICAL**: Every change MUST be pushed to GitHub after committing
- **CRITICAL**: All changes MUST be tested and committed to git before the task is done. MUST. Mandatory.
- Do NOT add Co-Authored-By: Claude <noreply@anthropic.com> to commit messages
- Commit changes with useful description explaining why things have changed
- do NOT add Co-Authored-By: Claude <noreply@anthropic.com> to commit messages
- release notes should be added to git and pushed
- commit changes with a useful description why things have changed

## Docker & CI/CD
- **Docker Images**: Available at `ghcr.io/rmoriz/alternator` 
- **Optimized Builds**: Use `Dockerfile.optimized` for production builds with pre-built binaries
- **Multi-arch Support**: Linux AMD64 and ARM64 platforms supported
- **CI/CD**: Automated builds on main/develop branches, releases on tags