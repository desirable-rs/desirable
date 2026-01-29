# AGENTS.md

This file provides guidelines for AI coding agents working on the desirable web framework.

## Build, Lint, and Test Commands

### Core Commands
```bash
# Build the library
cargo build --verbose

# Run all tests
cargo test --verbose

# Run a specific test
cargo test test_name -- --nocapture

# Run examples
cd examples && cargo run -p example-hello dev

# Check code formatting
cargo fmt --check

# Format code
cargo fmt

# Clippy linting
cargo clippy --all-targets --all-features -- -D warnings
```

### CI/CD
The project uses GitHub Actions (`.github/workflows/rust.yml`):
- Builds on Ubuntu latest
- Runs `cargo build --verbose` and `cargo test --verbose` on push/PR to main/develop

## Code Style Guidelines

### Formatting
- **Tabs**: 2 spaces (configured in `rustfmt.toml`)
- **Edition**: Rust 2024
- Run `cargo fmt` before committing

### Imports
- Group imports: std → external crates → crate modules
- Use `crate::` for internal imports
- Use `super::` for parent module imports
- Example:
```rust
use std::net::SocketAddr;
use std::sync::Arc;
use hyper::Error;
use crate::{Request, Response, Result};
```

### Naming Conventions
- **Structs/Types**: PascalCase (`Router`, `Server`)
- **Functions/methods**: snake_case (`new()`, `dispatch()`)
- **Variables**: snake_case (`remote_addr`, `path`)
- **Constants**: SCREAMING_SNAKE_CASE for global constants
- **Type aliases**: PascalCase (`HyperRequest`, `AnyResult`)
- **Error enum variants**: PascalCase (`Error::Hyper`, `Error::MissingParam`)

### Error Handling
- Use `thiserror` for the main `Error` enum (see `src/error.rs`)
- Use `anyhow::Result<T>` (`AnyResult<T>`) for fallible operations without specific error context
- Use `crate::Result<T>` (default `Response`) for API-level errors
- Provide user-friendly error messages with `#[error("...")]`
- Helper functions for common errors:
  - `missing_param(name: &str) -> Error`
  - `error_msg(msg: &str) -> Error`
  - `invalid_param(name, expected, err) -> Error`

### Type Definitions (`src/types.rs`)
- Default Result type is `Result<Response>` (shorthand for API endpoints)
- Use `AnyResult<T>` for operations needing anyhow's context
- Keep hyper types abstracted: `HyperRequest`, `HyperResponse`

### Module Organization
- All modules public in `lib.rs`: `pub mod module_name;`
- Public API re-exports: `pub use module::Type;`
- Follow the pattern: `pub mod error; pub mod fs; pub mod router;` etc.

### Async/Await
- Use `async-trait` for trait methods requiring async
- Avoid blocking in async contexts
- Prefer `Arc<dyn Trait>` for dynamic middleware dispatch

### Documentation
- Document public APIs with `///` doc comments
- Include error cases in function documentation
- Use `#[must_use]` for intentional functions like `new(addr: &str) -> Server`

### Testing
- Add `#[cfg(test)]` modules in source files
- Use `#[test]` attributes for unit tests
- Place integration tests in `tests/` directory when added

### Performance Notes
- Release profile uses LTO, opt-level="z", strip=true, codegen-units=1
- Use `Arc<Box<dyn Trait>>` pattern for shared trait objects (see middleware)
- Avoid cloning; prefer borrowing where possible
