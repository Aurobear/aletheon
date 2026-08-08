# Contributing to Aletheon

Thank you for your interest in contributing to Aletheon! This document provides guidelines and information about contributing to this project.

## Getting Started

1. Fork the repository on GitHub
2. Clone your fork locally
3. Create a feature branch from `dev` (use `auro/feat/` or `auro/fix/` prefix)
4. Make your changes
5. Push to your fork and submit a pull request

## Development Setup

### Prerequisites

- Rust 1.85+ (2021 edition)
- Cargo

Repository Cargo commands must run through `scripts/cargo-agent.sh`. The
wrapper provides a bounded shared target cache and serializes compilation
across concurrent worktrees.

The wrapper sizes compilation concurrency from total and currently available
memory (capped at eight), uses `sccache` and `mold` when they are installed, and
keeps incremental compilation enabled in the dev, test, release, and benchmark
profiles. Tagged release artifacts explicitly disable incremental codegen.
Override `CARGO_BUILD_JOBS` for a specific host, set
`ALETHEON_CARGO_FAST_LINKER=off` to disable mold, or set
`CARGO_PROFILE_TEST_DEBUG=1` when line-level test debug information is needed.
The 60 GiB target-capacity audit is throttled to once every 15 minutes so a
recursive size scan does not delay every no-op incremental command. Set
`ALETHEON_CARGO_TARGET_FORCE_SCAN=1` for an immediate audit or override
`ALETHEON_CARGO_TARGET_SCAN_INTERVAL_SEC` when operating under a tighter disk
budget.

Prefer crate- and target-scoped edit loops instead of enumerating every
workspace integration binary:

```bash
just dev executive
just test-lib executive
just test-one executive daemon_turn_engine
just lint-one executive
```

Rust source modules within one crate are not separate Cargo compilation units;
incremental rustc codegen handles changed units inside that crate. Use a new
crate boundary only when the code has a genuine stable architectural API, not
merely to split a file.

### Building

```bash
bash scripts/cargo-agent.sh build
```

### Running Tests

```bash
bash scripts/cargo-agent.sh test --workspace
```

### Linting

```bash
bash scripts/cargo-agent.sh clippy --workspace -- -D warnings
```

## Code Style

- Follow standard Rust formatting (use `bash scripts/cargo-agent.sh fmt --all`)
- Use meaningful variable and function names
- Add comments for complex logic
- Write documentation for public APIs

## Commit Messages

Use conventional commit format:

```
type(scope): description

[optional body]

[optional footer]
```

Types:
- `feat`: New feature
- `fix`: Bug fix
- `docs`: Documentation changes
- `style`: Code style changes (formatting, etc.)
- `refactor`: Code refactoring
- `test`: Adding tests
- `chore`: Maintenance tasks

## Pull Request Process

1. Ensure all tests pass
2. Update documentation if needed
3. Keep PRs focused on a single change
4. Reference any related issues
5. Wait for review and approval

## Reporting Issues

- Use GitHub Issues for bug reports and feature requests
- Provide as much detail as possible
- Include reproduction steps for bugs
- Check existing issues before creating new ones

## License

By contributing to Aletheon, you agree that your contributions will be licensed under the MIT License.
