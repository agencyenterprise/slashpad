# Contributing to Slashpad

Thanks for your interest in contributing to Slashpad!

## Getting started

See the [README](README.md) for installation and setup instructions. For architecture details and internal design decisions, see [CLAUDE.md](CLAUDE.md).

### Prerequisites

- [Rust](https://rustup.rs/) (stable)
- [Node.js](https://nodejs.org/) 18+
- macOS (the only supported platform currently)

### Development

```bash
npm install        # Install sidecar Node dependencies
cargo run          # Build and run in development mode
cargo check        # Fast type-check feedback loop
cargo clippy       # Lint
```

There is no automated test suite yet — manual smoke testing is the current workflow.

## Submitting changes

1. Fork the repo and create a branch
2. Make your changes
3. Run `cargo clippy` and fix any warnings
4. Open a pull request with a clear description of what changed and why

## Reporting issues

Open an issue on GitHub. Include:
- What you expected to happen
- What actually happened
- Steps to reproduce
- macOS version and Rust/Node versions
