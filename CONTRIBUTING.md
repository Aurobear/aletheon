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

- Rust 1.75+ (2021 edition)
- Cargo

### Building

```bash
cargo build
```

### Running Tests

```bash
cargo test
```

### Linting

```bash
cargo clippy -- -D warnings
```

## Code Style

- Follow standard Rust formatting (use `cargo fmt`)
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
