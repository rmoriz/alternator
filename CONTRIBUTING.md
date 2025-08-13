# Contributing to Alternator

Thank you for your interest in contributing to Alternator! This document provides guidelines and information for contributors.

## Code of Conduct

This project adheres to a code of conduct. By participating, you are expected to uphold this code. Please report unacceptable behavior to the project maintainers.

## How to Contribute

### Reporting Bugs

Before creating bug reports, please check the existing issues to avoid duplicates. When creating a bug report, include:

- **Clear title**: Descriptive summary of the issue
- **Environment**: OS, Rust version, Alternator version
- **Steps to reproduce**: Detailed steps to reproduce the behavior
- **Expected behavior**: What you expected to happen
- **Actual behavior**: What actually happened
- **Logs**: Relevant log output (sanitize any sensitive information)
- **Configuration**: Relevant configuration (remove sensitive tokens)

### Suggesting Features

Feature requests are welcome! Please provide:

- **Clear title**: Descriptive summary of the feature
- **Motivation**: Why this feature would be useful
- **Detailed description**: How the feature should work
- **Alternatives**: Other solutions you've considered
- **Additional context**: Screenshots, mockups, or examples

### Development Setup

1. **Prerequisites**
   ```bash
   # Install Rust (latest stable)
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   
   # Install required tools
   cargo install cargo-audit cargo-deny
   ```

2. **Clone and build**
   ```bash
   git clone https://github.com/rmoriz/alternator.git
   cd alternator
   cargo build
   ```

3. **Run tests**
   ```bash
   # Unit tests
   cargo test --lib
   
   # Integration tests
   cargo test --test integration_tests
   
   # All tests
   cargo test
   ```

4. **Code quality checks**
   ```bash
   # Format code
   cargo fmt
   
   # Lint code
   cargo clippy --all-targets --all-features
   
   # Security audit
   cargo audit
   ```

### Making Changes

1. **Fork** the repository on GitHub
2. **Create a branch** for your changes:
   ```bash
   git checkout -b feature/your-feature-name
   # or
   git checkout -b fix/issue-description
   ```

3. **Make your changes** following the guidelines below
4. **Test your changes** thoroughly
5. **Commit your changes** with clear messages
6. **Push to your fork** and submit a pull request

### Coding Standards

#### Rust Code Style

- **Formatting**: Use `cargo fmt` to format code
- **Linting**: Address all `cargo clippy` warnings
- **Documentation**: Add doc comments for public APIs
- **Error handling**: Use appropriate error types and handling
- **Testing**: Write unit tests for new functionality

#### Commit Messages

Follow conventional commit format:

```
type(scope): description

body (optional)

footer (optional)
```

Types:
- `feat`: New feature
- `fix`: Bug fix
- `docs`: Documentation changes
- `style`: Code style changes (formatting, etc.)
- `refactor`: Code refactoring
- `test`: Adding or modifying tests
- `chore`: Maintenance tasks

Examples:
```
feat(media): add support for AVIF image format
fix(mastodon): handle WebSocket disconnection gracefully
docs(readme): update installation instructions
test(toot): add race condition test cases
```

#### Code Organization

- **Modules**: Keep modules focused and cohesive
- **Functions**: Keep functions small and single-purpose
- **Error handling**: Use `Result` types consistently
- **Async**: Use async/await appropriately
- **Dependencies**: Minimize external dependencies

### Testing Guidelines

#### Unit Tests

- Test all public functions
- Test error conditions
- Use descriptive test names
- Include edge cases

```rust
#[test]
fn test_media_processor_filters_unsupported_formats() {
    let processor = MediaProcessor::with_default_config();
    let media = create_test_media("video/mp4");
    let result = processor.filter_processable_media(&[media]);
    assert_eq!(result.len(), 0);
}
```

#### Integration Tests

- Test component interactions
- Test configuration scenarios
- Test error recovery mechanisms

#### Test Data

- Use realistic test data
- Sanitize any sensitive information
- Create reusable test utilities

### Documentation

#### Code Documentation

- Add doc comments for all public items
- Include examples in doc comments
- Document error conditions
- Keep documentation up to date

```rust
/// Processes media attachments and generates descriptions
/// 
/// # Arguments
/// 
/// * `media` - The media attachments to process
/// 
/// # Returns
/// 
/// Returns `Ok(())` if processing succeeds, or an error if processing fails
/// 
/// # Errors
/// 
/// This function will return an error if:
/// - The media format is unsupported
/// - The OpenRouter API request fails
/// - Network connectivity issues occur
/// 
/// # Examples
/// 
/// ```rust
/// let processor = MediaProcessor::new();
/// let result = processor.process_media(&media_attachments).await?;
/// ```
pub async fn process_media(&self, media: &[MediaAttachment]) -> Result<(), MediaError> {
    // Implementation
}
```

#### User Documentation

- Update README.md for user-facing changes
- Update configuration examples
- Add troubleshooting information
- Include usage examples

### Pull Request Process

1. **Ensure CI passes**: All tests, formatting, and linting must pass
2. **Update documentation**: Include any necessary documentation updates
3. **Update CHANGELOG.md**: Add an entry describing your changes
4. **Request review**: Request review from maintainers
5. **Address feedback**: Respond to review comments promptly

#### Pull Request Template

```markdown
## Description
Brief description of changes

## Type of Change
- [ ] Bug fix
- [ ] New feature
- [ ] Breaking change
- [ ] Documentation update

## Testing
- [ ] Unit tests added/updated
- [ ] Integration tests added/updated
- [ ] Manual testing completed

## Checklist
- [ ] Code follows project style guidelines
- [ ] Self-review completed
- [ ] Documentation updated
- [ ] CHANGELOG.md updated
```

### Release Process

Releases are managed by project maintainers:

1. Update version in `Cargo.toml`
2. Update `CHANGELOG.md`
3. Create release tag
4. GitHub Actions automatically builds and publishes releases

### Development Workflow

#### Feature Development

1. **Plan**: Discuss feature design in an issue
2. **Implement**: Create feature branch and implement
3. **Test**: Add comprehensive tests
4. **Document**: Update documentation
5. **Review**: Submit pull request for review

#### Bug Fixes

1. **Reproduce**: Create a test that reproduces the bug
2. **Fix**: Implement the fix
3. **Verify**: Ensure the test passes
4. **Regress**: Add regression tests if needed

#### Refactoring

1. **Identify**: Document what needs refactoring and why
2. **Plan**: Break large refactors into smaller pieces
3. **Test**: Ensure existing behavior is preserved
4. **Review**: Get feedback on architectural changes

### Getting Help

- **Documentation**: Check README.md and inline documentation
- **Issues**: Search existing issues for similar problems
- **Discussions**: Use GitHub Discussions for questions
- **Code review**: Ask for feedback in pull requests

### Security

- **Vulnerabilities**: Report security issues via email, not public issues
- **Dependencies**: Keep dependencies updated and secure
- **Secrets**: Never commit API keys, tokens, or other secrets
- **Audit**: Run `cargo audit` regularly

### Performance

- **Benchmarks**: Include benchmarks for performance-critical code
- **Profiling**: Use profiling tools to identify bottlenecks
- **Memory**: Monitor memory usage and avoid leaks
- **Async**: Use async/await efficiently

### Licensing

By contributing to Alternator, you agree that your contributions will be licensed under the AGPL-3.0 License.

### Recognition

Contributors will be recognized in:
- GitHub contributors list
- Release notes for significant contributions
- Project documentation where appropriate

Thank you for contributing to Alternator! 🎉