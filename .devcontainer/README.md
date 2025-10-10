# Development Container Configuration

This directory contains the development container configuration for Turborepo. The devcontainer provides a fully configured development environment with all required dependencies.

## What's Included

### Core Dependencies
- **Rust**: Nightly toolchain (nightly-2025-06-20) with rustfmt, clippy, and llvm-tools-preview
- **Node.js**: Version 22.x (as specified in package.json engines)
- **pnpm**: Version 8.14.0 (as specified in package.json packageManager)

### Build Tools
- **protoc**: Protocol Buffers compiler
- **capnp**: Cap'n Proto serialization framework  
- **LLD**: LLVM linker (required for Linux builds)

### Development Tools
- **jq**: JSON processor (required for tests)
- **zstd**: Compression library (required for tests)
- **cargo-llvm-cov**: Coverage reporting tool
- **cargo-watch**: File watcher for Cargo
- **cargo-edit**: Cargo subcommands for dependency management

### VS Code Extensions
- Rust Analyzer with optimized settings
- ESLint and Prettier for JavaScript/TypeScript
- Tailwind CSS IntelliSense
- GitLens and GitHub integration
- Markdown support
- And more development productivity tools

## Quick Start

1. Open the repository in VS Code
2. When prompted, click "Reopen in Container" or run "Dev Containers: Reopen in Container"
3. Wait for the container to build and the post-create script to run
4. Start developing!

## Post-Create Setup

The `post-create.sh` script automatically:
- Installs Node.js dependencies with `pnpm install`
- Verifies Rust toolchain installation
- Builds the project with `cargo build`
- Displays helpful quick-start commands

## Useful Commands

After the container is ready, you can use:

```bash
# Rust development
cargo build                    # Build the project
cargo test                     # Run unit tests  
cargo coverage                 # Run tests with coverage
cargo fmt                      # Format Rust code
cargo clippy                   # Run linter

# Node.js development
pnpm install                   # Install dependencies
pnpm build                     # Build all packages
pnpm test                      # Run integration tests

# Combined workflows
pnpm -- turbo run build        # Build all packages using Turbo
pnpm -- turbo run test         # Run all tests using Turbo
```

## Configuration Notes

- The container runs as the `vscode` user for security
- Docker-in-Docker is enabled for containerized workflows
- Git and GitHub CLI are pre-installed
- The Rust toolchain matches the `rust-toolchain.toml` specification
- Node.js version matches the `engines` field in `package.json`
- All dependencies align with requirements in `CONTRIBUTING.md`

## Troubleshooting

If you encounter issues:

1. **Container build fails**: Check that Docker has sufficient resources allocated
2. **Rust toolchain issues**: The post-create script will verify the installation
3. **Node.js dependency issues**: Try running `pnpm install --frozen-lockfile`
4. **Permission issues**: All operations should work as the `vscode` user

For more detailed information, see the main [CONTRIBUTING.md](../CONTRIBUTING.md) file.