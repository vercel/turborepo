# Turborepo Evals

Evaluates the quality and correctness of Turborepo configuration and usage against popular AI models.

## Quick Start

### Prerequisites

- [Bun](https://bun.sh) - JavaScript runtime & package manager
- [pnpm](https://pnpm.io) - Package manager (for shared dependency management)

### Local Setup

```bash
# Navigate to turbo-evals
cd turbo-evals

# Install dependencies
pnpm install

# Show help
bun cli.ts --help
```

### Environment Variables

Set up your API keys:

```bash
# For LLM-based evals
export BRAINTRUST_API_KEY="your-braintrust-key"
export AI_GATEWAY_API_KEY="your-ai-gateway-key"

# For Claude Code evals
export ANTHROPIC_API_KEY="your-anthropic-key"
```

**Note:** The `--dry` flag is recommended for testing as it runs evaluations locally without uploading results to Braintrust

## Usage

### CLI Commands

#### LLM-based Evals

Run evals using various LLM models (configured in `lib/models.ts`):

```bash
# Show help and all available options
bun cli.ts --help

# Run a specific eval (uploads results to Braintrust)
bun cli.ts --eval 001-add-cache-outputs

# Run eval locally without Braintrust upload (recommended for testing)
bun cli.ts --dry --eval 001-add-cache-outputs

# Run all evals in parallel
bun cli.ts --all --dry

# Run with multiple worker threads for better performance
bun cli.ts --all --dry --threads 4

# Run with all models (default: only first model)
bun cli.ts --dry --eval 001-add-cache-outputs --all-models

# Debug mode - keep output folders for inspection
bun cli.ts --dry --debug --eval 001-add-cache-outputs

# Verbose output - see detailed logs during execution
bun cli.ts --dry --verbose --eval 001-add-cache-outputs

# Create a new eval from template
bun cli.ts --create --name "my-new-eval" --prompt "Add caching for the lint task"
```

#### Claude Code Evals

Run evals using Claude Code (AI coding agent):

```bash
# Run a specific eval with Claude Code
bun claude-code-cli.ts --eval 001-add-cache-outputs

# Or use the main CLI with --claude-code flag
bun cli.ts --eval 001-add-cache-outputs --claude-code

# Run all evals with Claude Code
bun claude-code-cli.ts --all

# With custom timeout (default: 600000ms = 10 minutes)
bun claude-code-cli.ts --eval 001-add-cache-outputs --timeout 900000

# Verbose output
bun claude-code-cli.ts --eval 001-add-cache-outputs --verbose

# Debug mode - keep output folders
bun claude-code-cli.ts --eval 001-add-cache-outputs --debug
```

## How it works

### Eval structure

Each eval consists of:

1. **Input Directory** (`input/`): A Turborepo monorepo in its initial state with failing tests
2. **Prompt File** (`prompt.md`): Contains the prompt text for the LLM
3. **Output Directory** (`output/`): Generated during eval run, contains LLM-modified project

### Evaluation Process

1. **Copy**: Input directory is copied to output directory
2. **Analyze**: LLM reads all project files and receives the eval prompt
3. **Generate**: LLM provides changes as complete file contents
4. **Apply**: Changes are applied to the output directory
5. **Validate**: Project runs `turbo run build test lint` to validate
6. **Score**: Success is measured by turbo task results (binary 1.0/0.0)
7. **Cleanup**: Output directory is removed (unless --debug)

### Dry Run Mode

Use `--dry` flag to run evaluations locally without uploading results to Braintrust:

- Results are displayed in the CLI with detailed pass/fail information
- Shows timing for build, lint, and test phases
- Displays debug output for failed evaluations
- Useful for quick testing and development

### Table Output

Results are displayed in a summary table:

```
| Eval                     | Status   | Build | Lint  | Tests |
|--------------------------|----------|-------|-------|-------|
| 001-add-cache-outputs    | PASS     |       |       |       |
| 002-add-workspace-dep    | FAIL     |       |       |       |
```

### Debug Mode

Use `--debug` flag to preserve output folders for inspection:

```bash
bun cli.ts --dry --debug --eval 001-add-cache-outputs
```

This keeps the `output-dry/` folders after completion, allowing you to:

- Inspect the AI-generated changes
- Debug turbo configuration issues
- Understand why an eval failed

### Creating New Evals

```bash
bun cli.ts --create --name "my-new-eval" --prompt "Configure remote caching"
```

The CLI will:

1. Create a new numbered directory under `evals/`
2. Copy the `template/` directory to create the `input/` state
3. Create a `prompt.md` file with your prompt text

## Eval Categories

Turborepo evals test various aspects of monorepo configuration:

### Configuration Evals

- Adding/modifying tasks in `turbo.json`
- Setting up task dependencies (`dependsOn`)
- Configuring inputs and outputs
- Environment variable handling

### Caching Evals

- Configuring cache outputs
- Setting up remote caching
- Handling cache invalidation

### Workspace Evals

- Package filtering
- Workspace dependencies
- Root tasks vs package tasks

### Advanced Evals

- Watch mode configuration
- Persistent tasks
- Task parallelization

## Models

Currently configured models (in `lib/models.ts`):

- OpenAI GPT-5
- Anthropic Claude 4 Sonnet
- Google Gemini 2.5 Flash
- XAI Grok 4

To add or modify models, edit the `MODELS` array in `lib/models.ts`.

### Custom Model Providers

To add your own custom model providers, see [Custom Model Provider Integration](lib/providers/README.md) for detailed instructions.

## Template

The `template/` directory contains a basic Turborepo monorepo that serves as the starting point for all new evals. It includes:

- Root `turbo.json` with basic task configuration
- `packages/` directory with sample packages
- TypeScript configuration
- Testing setup with Vitest
- A failing test that evals should fix

## Dependency Management

This project uses a **shared dependency system** where all evals use the same `node_modules` for consistency.

### Quick Start

```bash
# Update dependencies across all evals
# 1. Edit template/package.json with new versions
# 2. Run:
bun scripts/sync-templates.ts
```

The sync script will:

- Copy template files to all eval `input/` directories
- Compare `evals/package.json` with the template
- If different: remove `evals/node_modules`, update, and run `pnpm install`
