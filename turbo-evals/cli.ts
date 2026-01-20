#!/usr/bin/env bun

import { spawn } from "child_process";
import fs from "fs/promises";
import { cpus, freemem, totalmem } from "os";
import path from "path";
import {
  type ClaudeCodeResult,
  runClaudeCodeEval
} from "./lib/claude-code-runner";
import { createNewEval, runEval } from "./lib/eval-runner";
import { formatClaudeCodeResultsTable } from "./lib/format-results";
import { MODELS } from "./lib/models";

let globalProgressTracker: ProgressTracker | null = null;
let globalDebugMode: boolean = false;

// Memory management utilities
function formatMemory(bytes: number): string {
  return `${Math.round(bytes / 1024 / 1024)}MB`;
}

function checkAvailableMemory(minRequiredMB: number = 500): {
  hasEnough: boolean;
  available: number;
  total: number;
  percentage: number;
} {
  const freeBytes = freemem();
  const totalBytes = totalmem();
  const availableMB = freeBytes / 1024 / 1024;
  const percentage = Math.round((freeBytes / totalBytes) * 100);

  return {
    hasEnough: availableMB >= minRequiredMB,
    available: Math.round(availableMB),
    total: Math.round(totalBytes / 1024 / 1024),
    percentage
  };
}

// Cleanup function to stop progress tracker and optionally clean shared folder
async function cleanup(
  _skipCleanup: boolean = true,
  cleanShared: boolean = false
) {
  const skipCleanup = true;
  if (globalProgressTracker) {
    globalProgressTracker.stop();
    globalProgressTracker = null;
  }

  if (skipCleanup) {
    console.log("🐛 Debug mode: Preserving state for inspection");
    return;
  }

  // Clean up output-dry folders from previous runs
  try {
    const evalsDir = path.join(process.cwd(), "evals");
    const evalEntries = await fs.readdir(evalsDir, { withFileTypes: true });

    for (const entry of evalEntries) {
      if (entry.isDirectory() && /^\d+/.test(entry.name)) {
        const evalPath = path.join(evalsDir, entry.name);
        const evalContents = await fs.readdir(evalPath, {
          withFileTypes: true
        });

        // Remove all output-dry* directories
        for (const item of evalContents) {
          if (item.isDirectory() && item.name.startsWith("output-dry")) {
            const outputDirPath = path.join(evalPath, item.name);
            try {
              await fs.rm(outputDirPath, { recursive: true, force: true });
            } catch (error) {
              // Ignore errors if folder doesn't exist or can't be removed
            }
          }
        }
      }
    }
  } catch (error) {
    // Ignore errors if evals directory doesn't exist
  }

  // Only clean shared folder if requested
  if (cleanShared) {
    try {
      const evalsDir = path.join(process.cwd(), "evals");
      const sharedDir = path.join(evalsDir, ".shared");
      try {
        await fs.rm(sharedDir, { recursive: true, force: true });
        console.log("🧹 Cleaned shared node_modules");
      } catch (error) {
        // Ignore errors if folder doesn't exist
      }
    } catch (error) {
      // Ignore errors if evals directory doesn't exist
    }
  }
}

// Cleanup on process termination
process.on("SIGINT", async () => {
  console.log("\n\n🧹 Cleaning up...");
  await cleanup(globalDebugMode);
  process.exit(0);
});

process.on("SIGTERM", async () => {
  console.log("\n\n🧹 Cleaning up...");
  await cleanup(globalDebugMode);
  process.exit(0);
});

process.on("uncaughtException", async (error) => {
  console.error("Uncaught exception:", error);
  await cleanup(globalDebugMode);
  process.exit(1);
});

process.on("unhandledRejection", async (reason) => {
  console.error("Unhandled rejection:", reason);
  await cleanup(globalDebugMode);
  process.exit(1);
});

// Thread-safe progress tracker for parallel execution
export class ProgressTracker {
  private statuses = new Map<string, "running" | "success" | "fail">();
  private startTime = Date.now();
  private interval: NodeJS.Timeout | null = null;
  private totalEvals: number = 0;

  start(evalPaths: string[]) {
    this.totalEvals = evalPaths.length;

    // Initialize all evals as running
    evalPaths.forEach((evalPath) => {
      this.statuses.set(evalPath, "running");
    });

    console.log("⚡ Running evals...\n");

    // Show periodic progress updates
    this.interval = setInterval(() => {
      this.showProgress();
    }, 2000);
  }

  succeed(id: string) {
    this.statuses.set(id, "success");
    console.log(`✅ ${id}`);

    if (this.allComplete()) {
      this.stop();
    }
  }

  fail(id: string) {
    this.statuses.set(id, "fail");
    console.log(`❌ ${id}`);

    if (this.allComplete()) {
      this.stop();
    }
  }

  private allComplete(): boolean {
    return Array.from(this.statuses.values()).every(
      (status) => status !== "running"
    );
  }

  private showProgress() {
    const completed = Array.from(this.statuses.values()).filter(
      (status) => status !== "running"
    ).length;
    const successful = Array.from(this.statuses.values()).filter(
      (status) => status === "success"
    ).length;
    const elapsed = Math.round((Date.now() - this.startTime) / 1000);

    console.log(
      `📊 Progress: ${completed}/${this.totalEvals} complete (${successful} passed) - ${elapsed}s elapsed`
    );
  }

  stop() {
    if (this.interval) {
      clearInterval(this.interval);
      this.interval = null;
    }

    const completed = Array.from(this.statuses.values()).filter(
      (status) => status !== "running"
    ).length;
    const successful = Array.from(this.statuses.values()).filter(
      (status) => status === "success"
    ).length;
    const elapsed = Math.round((Date.now() - this.startTime) / 1000);

    console.log(
      `\n🏁 Final: ${completed}/${this.totalEvals} complete (${successful} passed) - ${elapsed}s total\n`
    );
  }
}

class Spinner {
  private interval: NodeJS.Timeout | null = null;
  private readonly frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
  private frameIndex = 0;

  start(text: string) {
    if (text) {
      process.stdout.write(`${text} ${this.frames[0]}`);
    } else {
      // For empty text, write on new line so it doesn't interfere with other output
      process.stdout.write(`\n ${this.frames[0]}`);
    }
    this.interval = setInterval(() => {
      this.frameIndex = (this.frameIndex + 1) % this.frames.length;
      if (text) {
        process.stdout.write(`\r${text} ${this.frames[this.frameIndex]}`);
      } else {
        // For empty text, use cursor up and clear line to update in place
        process.stdout.write(`\x1b[1A\r ${this.frames[this.frameIndex]}\n`);
      }
    }, 80);
  }

  succeed(text: string) {
    this.stop();
    process.stdout.write(`\r${text} ✅\n`);
  }

  fail(text: string) {
    this.stop();
    process.stdout.write(`\r${text} ❌\n`);
  }

  stop() {
    if (this.interval) {
      clearInterval(this.interval);
      this.interval = null;
      // Move cursor up and clear the spinner line
      process.stdout.write("\x1b[1A\r" + " ".repeat(80) + "\r");
    }
  }
}

// Simple argument parser for Bun compatibility
function parseCliArgs(args: string[]) {
  const values: Record<string, any> = {};
  const positionals: string[] = [];

  for (let i = 0; i < args.length; i++) {
    const arg = args[i];

    if (arg === "-h" || arg === "--help") {
      values.help = true;
    } else if (arg === "-c" || arg === "--create") {
      values.create = true;
    } else if (arg === "-a" || arg === "--all") {
      values.all = true;
    } else if (arg === "-d" || arg === "--dry") {
      values.dry = true;
    } else if (arg === "-v" || arg === "--verbose") {
      values.verbose = true;
    } else if (arg === "--debug") {
      values.debug = true;
    } else if (arg === "--all-models") {
      values["all-models"] = true;
    } else if (arg === "--agent-evals") {
      values["agent-evals"] = true;
    } else if (arg === "--claude-code") {
      values["claude-code"] = true;
    } else if (arg === "-e" || arg === "--eval") {
      values.eval = args[++i];
    } else if (arg === "--evals") {
      values.evals = args[++i];
    } else if (arg === "-n" || arg === "--name") {
      values.name = args[++i];
    } else if (arg === "-p" || arg === "--prompt") {
      values.prompt = args[++i];
    } else if (arg === "-t" || arg === "--threads") {
      values.threads = args[++i];
    } else if (arg === "--claude-timeout") {
      values["claude-timeout"] = args[++i];
    } else if (arg === "--api-key") {
      values["api-key"] = args[++i];
    } else if (arg === "--dev-server-cmd") {
      values["dev-server-cmd"] = args[++i];
    } else if (arg === "--dev-server-port") {
      values["dev-server-port"] = args[++i];
    } else if (arg === "--with-hooks") {
      values["with-hooks"] = args[++i];
    } else if (arg === "--with-visual-diff") {
      values["with-visual-diff"] = true;
    } else if (!arg.startsWith("-")) {
      positionals.push(arg);
    }
  }

  return { values, positionals };
}

const { values, positionals } = parseCliArgs(process.argv.slice(2));

function showHelp() {
  console.log(`
Turborepo Evals CLI

Usage:
  cli.ts [options] [eval-path]

Options:
  -h, --help              Show this help message
  -c, --create            Create a new eval
  -a, --all               Run all evals
  -e, --eval <path>       Run a specific eval by path
      --evals <paths>     Run multiple evals (comma-separated, e.g., "001,002,003")
  -n, --name <name>       Name for new eval (required with --create)
  -p, --prompt <prompt>   Prompt for new eval (required with --create)
  -d, --dry               Run eval locally without uploading to Braintrust
  -v, --verbose           Show detailed logs during eval execution
      --debug             Persist output folders for debugging (don't clean up)
  -t, --threads <num>     Number of worker threads (default: 1, max: CPU cores)
      --all-models        Run single eval with all models (default: only first model)
      --claude-code       Use Claude Code agent instead of LLM models
      --claude-timeout    Timeout for Claude Code in ms (default: 600000 = 10 minutes)
      --api-key <key>     Anthropic API key for Claude Code (or use ANTHROPIC_API_KEY env var)

Examples:
  # Run all evals with LLMs
  cli.ts --all

  # Run all evals with Claude Code
  cli.ts --all --claude-code

  # Run a specific eval with Claude Code
  cli.ts --eval 001-add-cache-outputs --claude-code

  # Run multiple specific evals
  cli.ts --evals 001-add-cache-outputs,002-add-workspace-dep

  # Run Claude Code eval with custom timeout and API key
  cli.ts --eval 001-add-cache-outputs --claude-code --claude-timeout 600000 --api-key your-key

  # Create a new eval
  cli.ts --create --name "my-new-eval" --prompt "Add caching for the lint task"

  # Run eval by positional argument
  cli.ts 001-add-cache-outputs

  # Run eval locally without Braintrust upload
  cli.ts --dry --eval 001-add-cache-outputs

  # Debug mode - keep output folders for inspection
  cli.ts --dry --debug --eval 001-add-cache-outputs

  # Use multiple worker threads for faster parallel execution
  cli.ts --all --dry --threads 4
`);
}

async function getAllEvals(agentEvalsOnly: boolean = false): Promise<string[]> {
  const evalsDir = path.join(process.cwd(), "evals");
  const entries = await fs.readdir(evalsDir, { withFileTypes: true });

  const evals: string[] = [];

  for (const entry of entries) {
    // Match evals based on filter
    const isAgentEval = /^agent-\d+/.test(entry.name);
    const isRegularEval = /^\d+/.test(entry.name);

    // Filter based on agentEvalsOnly flag
    const shouldInclude = agentEvalsOnly ? isAgentEval : isRegularEval;

    if (entry.isDirectory() && shouldInclude) {
      const evalPath = path.join(evalsDir, entry.name);
      // Check if it has both input/ directory and prompt.md
      const hasInput = await fs
        .stat(path.join(evalPath, "input"))
        .then((s) => s.isDirectory())
        .catch(() => false);
      const hasPrompt = await fs
        .stat(path.join(evalPath, "prompt.md"))
        .then((s) => s.isFile())
        .catch(() => false);

      if (hasInput && hasPrompt) {
        evals.push(entry.name);
      }
    }
  }

  return evals.sort();
}

function extractErrorSummary(output: string, maxLength: number = 0): string {
  if (!output) return "";

  // Look for common error patterns
  const lines = output.split("\n").filter((line) => line.trim());

  // Skip generic "Command failed" lines and look for actual error content
  const meaningfulLines = lines.filter(
    (line) =>
      !line.includes("Command failed: cd") &&
      !line.includes("execAsync") &&
      line.trim().length > 0
  );

  let errorLines: string[] = [];

  // Look for specific Next.js/build error patterns first
  for (let i = 0; i < meaningfulLines.length; i++) {
    const line = meaningfulLines[i];
    if (
      line.match(
        /error:|Error:|TypeError|SyntaxError|ReferenceError|Module not found|Cannot resolve|Failed to compile/i
      )
    ) {
      errorLines.push(line.trim());
      // Include context lines for better understanding
      for (let j = i + 1; j < Math.min(i + 5, meaningfulLines.length); j++) {
        const nextLine = meaningfulLines[j].trim();
        if (nextLine && !nextLine.match(/^(at |in |>|─|┌|└)/)) {
          errorLines.push(nextLine);
        }
      }
      break;
    }
  }

  // Look for linting errors if no build errors found
  if (errorLines.length === 0) {
    for (const line of meaningfulLines) {
      if (
        line.match(
          /✖|warning:|Expected|Missing|'.*' is not defined|Unused variable/i
        )
      ) {
        errorLines.push(line.trim());
        break;
      }
    }
  }

  // Fallback to meaningful lines if no specific patterns found
  if (errorLines.length === 0 && meaningfulLines.length > 0) {
    errorLines = meaningfulLines.slice(0, 5);
  }

  // Join multiple lines with proper formatting
  let result = errorLines.join("\n        ");

  // Clean up common prefixes
  result = result.replace(/^(error|Error):\s*/, "");

  // Only truncate if maxLength is specified and > 0
  if (maxLength > 0 && result.length > maxLength) {
    return result.substring(0, maxLength - 3) + "...";
  }

  return result;
}

async function displaySingleResult(
  evalPath: string,
  result: any,
  isDryRun: boolean,
  modelName?: string
) {
  console.log("\n📊 Results:\n" + "═".repeat(80));

  if (isDryRun) {
    // Check if we have multiple model results
    if (result?.modelResults) {
      // Multiple model results - display as table
      const models = result.modelResults.map((mr: any) => mr.model);
      const evalColWidth = Math.max(25, evalPath.length);
      const modelColWidth = 20;

      // Build header
      let header = `| ${"Eval".padEnd(evalColWidth)} |`;
      for (const model of models) {
        header += ` ${model.padEnd(modelColWidth)} |`;
      }
      console.log(header);

      // Build separator
      let separator = `|${"-".repeat(evalColWidth + 2)}|`;
      for (const _ of models) {
        separator += `${"-".repeat(modelColWidth + 2)}|`;
      }
      console.log(separator);

      // Build row
      let row = `| ${evalPath.padEnd(evalColWidth)} |`;
      const errors: any[] = [];

      for (const modelResult of result.modelResults) {
        if (modelResult.error) {
          row += ` ❌❌❌`.padEnd(modelColWidth + 1) + "|";
          errors.push({
            model: modelResult.model,
            error: modelResult.error
          });
        } else {
          const buildSuccess =
            modelResult.result?.evaluationResults?.buildSuccess ?? false;
          const lintSuccess =
            modelResult.result?.evaluationResults?.lintSuccess ?? false;
          const testSuccess =
            modelResult.result?.evaluationResults?.testSuccess ?? false;

          const build = buildSuccess ? "✅" : "❌";
          const lint = lintSuccess ? "✅" : "❌";
          const test = testSuccess ? "✅" : "❌";

          row += ` ${build}${lint}${test}`.padEnd(modelColWidth + 1) + "|";

          // Collect errors
          if (!buildSuccess || !lintSuccess || !testSuccess) {
            const error: any = { model: modelResult.model };
            if (
              !buildSuccess &&
              modelResult.result?.evaluationResults?.buildOutput
            ) {
              error.buildError = extractErrorSummary(
                modelResult.result.evaluationResults.buildOutput,
                0
              );
            }
            if (
              !lintSuccess &&
              modelResult.result?.evaluationResults?.lintOutput
            ) {
              error.lintError = extractErrorSummary(
                modelResult.result.evaluationResults.lintOutput,
                0
              );
            }
            if (
              !testSuccess &&
              modelResult.result?.evaluationResults?.testOutput
            ) {
              error.testError = extractErrorSummary(
                modelResult.result.evaluationResults.testOutput,
                0
              );
            }
            errors.push(error);
          }
        }
      }

      console.log(row);
      console.log("\n📋 Legend: ✅✅✅ = Build/Lint/Test");

      // Display errors if any
      if (errors.length > 0) {
        console.log("\n❌ Error Details:");
        console.log("─".repeat(80));
        for (const error of errors) {
          console.log(`\n${error.model}:`);
          if (error.error) {
            console.log(`  Error: ${error.error}`);
          } else {
            if (error.buildError) {
              console.log(`  Build:  ${error.buildError}`);
            }
            if (error.lintError) {
              console.log(`  Lint:   ${error.lintError}`);
            }
            if (error.testError) {
              console.log(`  Tests:  ${error.testError}`);
            }
          }
        }
      }
    } else {
      // Single model result - display as table with one model
      const evalColWidth = Math.max(25, evalPath.length);
      const modelColWidth = 20;
      const displayModelName = modelName || MODELS[0]?.name || "Model";

      // Build header
      const header = `| ${"Eval".padEnd(
        evalColWidth
      )} | ${displayModelName.padEnd(modelColWidth)} |`;
      console.log(header);

      // Build separator
      const separator = `|${"-".repeat(evalColWidth + 2)}|${"-".repeat(
        modelColWidth + 2
      )}|`;
      console.log(separator);

      // Build row
      const buildSuccess = result?.evaluationResults?.buildSuccess ?? false;
      const lintSuccess = result?.evaluationResults?.lintSuccess ?? false;
      const testSuccess = result?.evaluationResults?.testSuccess ?? false;

      const build = buildSuccess ? "✅" : "❌";
      const lint = lintSuccess ? "✅" : "❌";
      const test = testSuccess ? "✅" : "❌";

      // Emojis take 2 character widths each, so pad accordingly
      // 3 emojis = 6 visual chars, so we need (modelColWidth - 3) spaces to reach modelColWidth visual chars
      const emojiString = build + lint + test;
      const paddingNeeded = modelColWidth - 3; // 3 emojis counted as 3 chars but display as 6
      const row = `| ${evalPath.padEnd(evalColWidth)} | ${emojiString.padEnd(
        paddingNeeded
      )} |`;
      console.log(row);
      console.log("\n📋 Legend: ✅✅✅ = Build/Lint/Test");

      // Display errors if any
      if (!buildSuccess || !lintSuccess || !testSuccess) {
        console.log("\n❌ Error Details:");
        console.log("─".repeat(80));
        if (!buildSuccess && result?.evaluationResults?.buildOutput) {
          const errorSummary = extractErrorSummary(
            result.evaluationResults.buildOutput,
            0
          );
          if (errorSummary) {
            console.log(`Build:  ${errorSummary}`);
          }
        }
        if (!lintSuccess && result?.evaluationResults?.lintOutput) {
          const errorSummary = extractErrorSummary(
            result.evaluationResults.lintOutput,
            0
          );
          if (errorSummary) {
            console.log(`Lint:   ${errorSummary}`);
          }
        }
        if (!testSuccess && result?.evaluationResults?.testOutput) {
          const errorSummary = extractErrorSummary(
            result.evaluationResults.testOutput,
            0
          );
          if (errorSummary) {
            console.log(`Tests:  ${errorSummary}`);
          }
        }
      }
    }
  } else {
    // For Braintrust runs, show eval score if available
    // For Claude Code runs (no Braintrust), check build/lint/test success
    const evalScore = result?.scores?.eval_score?.score;
    const passed =
      typeof evalScore === "number"
        ? evalScore >= 1
        : ((result?.evaluationResults?.buildSuccess &&
            result?.evaluationResults?.lintSuccess &&
            result?.evaluationResults?.testSuccess) ??
          false);

    const evalColWidth = Math.max(25, evalPath.length);
    const modelColWidth = 20;

    // Build header
    console.log(
      `| ${"Eval".padEnd(evalColWidth)} | ${"Braintrust".padEnd(
        modelColWidth
      )} |`
    );
    console.log(
      `|${"-".repeat(evalColWidth + 2)}|${"-".repeat(modelColWidth + 2)}|`
    );

    const emoji = passed ? "✅✅✅" : "❌❌❌";
    console.log(
      `| ${evalPath.padEnd(evalColWidth)} | ${emoji.padEnd(modelColWidth)} |`
    );

    console.log("\n📋 Legend: ✅✅✅ = Build/Lint/Test");

    // Show experiment URL if available
    if (result?.experimentUrl) {
      console.log(`\n🔗 Experiment: ${result.experimentUrl}`);
    }
  }

  console.log("═".repeat(80));
}

function transformToEvalResults(results: PromiseSettledResult<any>[]): Array<{
  evalPath: string;
  result: {
    buildSuccess: boolean;
    lintSuccess: boolean;
    testSuccess: boolean;
    duration?: number;
  };
}> {
  return results.map((result) => {
    if (result.status === "fulfilled" && result.value.status === "success") {
      const { evalPath, result: evalResult } = result.value;
      return {
        evalPath,
        result: {
          buildSuccess: evalResult?.evaluationResults?.buildSuccess ?? false,
          lintSuccess: evalResult?.evaluationResults?.lintSuccess ?? false,
          testSuccess: evalResult?.evaluationResults?.testSuccess ?? false,
          duration: evalResult?.duration
        }
      };
    } else {
      const evalPath =
        result.status === "fulfilled" ? result.value.evalPath : "Unknown";
      return {
        evalPath,
        result: {
          buildSuccess: false,
          lintSuccess: false,
          testSuccess: false
        }
      };
    }
  });
}

async function displayResultsTable(
  results: PromiseSettledResult<any>[],
  isDryRun: boolean
) {
  const totalTests = results.length;
  console.log(`\n📊 Results Summary (${totalTests} Tests):`);
  console.log("═".repeat(120));

  if (isDryRun) {
    // Check if we have multi-model results
    const firstResult = results.find((r) => r.status === "fulfilled")?.value;
    const hasMultiModelResults =
      firstResult?.result?.modelResults !== undefined;

    if (hasMultiModelResults) {
      // Extract unique model names from results
      const modelNames = new Set<string>();
      for (const result of results) {
        if (
          result.status === "fulfilled" &&
          result.value.result?.modelResults
        ) {
          for (const modelResult of result.value.result.modelResults) {
            modelNames.add(modelResult.model);
          }
        }
      }
      const models = Array.from(modelNames);

      // Calculate column widths
      const evalColWidth = Math.max(
        25,
        ...results.map((r) =>
          r.status === "fulfilled" ? r.value.evalPath.length : 0
        )
      );
      const modelColWidth = 20; // Fixed width for model columns

      // Build header
      let header = `| ${"Eval".padEnd(evalColWidth)} |`;
      for (const model of models) {
        header += ` ${model.padEnd(modelColWidth)} |`;
      }
      console.log(header);

      // Build separator
      let separator = `|${"-".repeat(evalColWidth + 2)}|`;
      for (const _ of models) {
        separator += `${"-".repeat(modelColWidth + 2)}|`;
      }
      console.log(separator);

      // Build rows
      const failedEvals: Array<{
        evalPath: string;
        model: string;
        buildError?: string;
        lintError?: string;
        testError?: string;
      }> = [];

      for (const result of results) {
        if (
          result.status === "fulfilled" &&
          result.value.status === "success"
        ) {
          const { evalPath, result: evalResult } = result.value;
          let row = `| ${evalPath.padEnd(evalColWidth)} |`;

          // For each model, show build/lint/test status
          for (const modelName of models) {
            const modelResult = evalResult.modelResults?.find(
              (mr: any) => mr.model === modelName
            );

            if (modelResult) {
              const buildSuccess =
                modelResult.result?.evaluationResults?.buildSuccess ?? false;
              const lintSuccess =
                modelResult.result?.evaluationResults?.lintSuccess ?? false;
              const testSuccess =
                modelResult.result?.evaluationResults?.testSuccess ?? false;

              // Show 3 emojis for build/lint/test
              const build = buildSuccess ? "✅" : "❌";
              const lint = lintSuccess ? "✅" : "❌";
              const test = testSuccess ? "✅" : "❌";

              row += ` ${build}${lint}${test}`.padEnd(modelColWidth + 1) + "|";

              // Collect errors
              if (!buildSuccess || !lintSuccess || !testSuccess) {
                const errors: any = { evalPath, model: modelName };
                if (
                  !buildSuccess &&
                  modelResult.result?.evaluationResults?.buildOutput
                ) {
                  errors.buildError = extractErrorSummary(
                    modelResult.result.evaluationResults.buildOutput,
                    0
                  );
                }
                if (
                  !lintSuccess &&
                  modelResult.result?.evaluationResults?.lintOutput
                ) {
                  errors.lintError = extractErrorSummary(
                    modelResult.result.evaluationResults.lintOutput,
                    0
                  );
                }
                if (
                  !testSuccess &&
                  modelResult.result?.evaluationResults?.testOutput
                ) {
                  errors.testError = extractErrorSummary(
                    modelResult.result.evaluationResults.testOutput,
                    0
                  );
                }
                failedEvals.push(errors);
              }
            } else {
              row += ` ⚫⚫⚫`.padEnd(modelColWidth + 1) + "|"; // No results
            }
          }

          console.log(row);
        } else {
          const evalPath =
            result.status === "fulfilled" ? result.value.evalPath : "Unknown";
          let row = `| ${evalPath.padEnd(evalColWidth)} |`;

          // Show all fails for error cases
          for (const _ of models) {
            row += ` ❌❌❌`.padEnd(modelColWidth + 1) + "|";
          }
          console.log(row);

          // Add generic error for all models
          for (const model of models) {
            failedEvals.push({
              evalPath,
              model,
              buildError:
                result.status === "rejected"
                  ? result.reason?.message || "Unknown error"
                  : "Eval execution failed"
            });
          }
        }
      }

      // Add score summary rows
      console.log(separator);

      // Calculate detailed scores for each model
      const modelScores = new Map<
        string,
        {
          passed: number;
          total: number;
          build: number;
          lint: number;
          test: number;
        }
      >();

      for (const model of models) {
        modelScores.set(model, {
          passed: 0,
          total: 0,
          build: 0,
          lint: 0,
          test: 0
        });
      }

      // Count passes and totals for each model
      for (const result of results) {
        if (
          result.status === "fulfilled" &&
          result.value.result?.modelResults
        ) {
          for (const modelResult of result.value.result.modelResults) {
            const score = modelScores.get(modelResult.model);
            if (score) {
              score.total++;
              const buildSuccess =
                modelResult.result?.evaluationResults?.buildSuccess ?? false;
              const lintSuccess =
                modelResult.result?.evaluationResults?.lintSuccess ?? false;
              const testSuccess =
                modelResult.result?.evaluationResults?.testSuccess ?? false;

              if (buildSuccess) score.build++;
              if (lintSuccess) score.lint++;
              if (testSuccess) score.test++;

              if (buildSuccess && lintSuccess && testSuccess) {
                score.passed++;
              }
            }
          }
        } else {
          // For failed evals, increment total for all models but don't increment any passes
          for (const model of models) {
            const score = modelScores.get(model);
            if (score) {
              score.total++;
            }
          }
        }
      }

      // Overall score row with build/lint/test breakdown
      let overallRow = `| ${"Overall (B/L/T)".padEnd(evalColWidth)} |`;
      for (const model of models) {
        const score = modelScores.get(model);
        if (score) {
          const buildPct =
            score.total > 0 ? Math.round((score.build / score.total) * 100) : 0;
          const lintPct =
            score.total > 0 ? Math.round((score.lint / score.total) * 100) : 0;
          const testPct =
            score.total > 0 ? Math.round((score.test / score.total) * 100) : 0;
          const scoreText = `${score.build}/${score.lint}/${score.test} (${buildPct}%, ${lintPct}%, ${testPct}%)`;
          overallRow += ` ${scoreText.padEnd(modelColWidth)} |`;
        }
      }
      console.log(overallRow);

      // Legend
      console.log("\n📋 Legend: ✅✅✅ = Build/Lint/Test");

      // Display error summaries
      if (failedEvals.length > 0) {
        console.log("\n❌ Error Summaries:");
        console.log("─".repeat(120));

        // Group errors by eval
        const errorsByEval = new Map<string, typeof failedEvals>();
        for (const error of failedEvals) {
          if (!errorsByEval.has(error.evalPath)) {
            errorsByEval.set(error.evalPath, []);
          }
          errorsByEval.get(error.evalPath)!.push(error);
        }

        for (const [evalPath, errors] of errorsByEval) {
          console.log(`\n${evalPath}:`);
          for (const error of errors) {
            console.log(`  ${error.model}:`);
            if (error.buildError) {
              console.log(`    Build:  ${error.buildError}`);
            }
            if (error.lintError) {
              console.log(`    Lint:   ${error.lintError}`);
            }
            if (error.testError) {
              console.log(`    Tests:  ${error.testError}`);
            }
          }
        }
      }
    } else {
      // Single model results - use unified formatting
      const transformedResults = transformToEvalResults(results);
      console.log(formatClaudeCodeResultsTable(transformedResults));
      console.log("\n📋 Legend: ✅✅✅ = Build/Lint/Test");

      // Collect and display error details
      const failedEvals: Array<{
        evalPath: string;
        buildError?: string;
        lintError?: string;
        testError?: string;
      }> = [];

      for (const result of results) {
        if (
          result.status === "fulfilled" &&
          result.value.status === "success"
        ) {
          const { evalPath, result: evalResult } = result.value;
          const buildSuccess =
            evalResult?.evaluationResults?.buildSuccess ?? false;
          const lintSuccess =
            evalResult?.evaluationResults?.lintSuccess ?? false;
          const testSuccess =
            evalResult?.evaluationResults?.testSuccess ?? false;

          // Collect errors for failed evals
          if (!buildSuccess || !lintSuccess || !testSuccess) {
            const errors: any = { evalPath };
            if (!buildSuccess && evalResult?.evaluationResults?.buildOutput) {
              errors.buildError = extractErrorSummary(
                evalResult.evaluationResults.buildOutput,
                0
              );
            }
            if (!lintSuccess && evalResult?.evaluationResults?.lintOutput) {
              errors.lintError = extractErrorSummary(
                evalResult.evaluationResults.lintOutput,
                0
              );
            }
            if (!testSuccess && evalResult?.evaluationResults?.testOutput) {
              errors.testError = extractErrorSummary(
                evalResult.evaluationResults.testOutput,
                0
              );
            }
            failedEvals.push(errors);
          }
        } else {
          const evalPath =
            result.status === "fulfilled" ? result.value.evalPath : "Unknown";
          failedEvals.push({
            evalPath,
            buildError:
              result.status === "rejected"
                ? result.reason?.message || "Unknown error"
                : "Eval execution failed"
          });
        }
      }

      if (failedEvals.length > 0) {
        console.log("\n❌ Error Summaries:");
        console.log("─".repeat(80));

        for (const failed of failedEvals) {
          console.log(`\n${failed.evalPath}:`);
          if (failed.buildError) {
            console.log(`  Build:  ${failed.buildError}`);
          }
          if (failed.lintError) {
            console.log(`  Lint:   ${failed.lintError}`);
          }
          if (failed.testError) {
            console.log(`  Tests:  ${failed.testError}`);
          }
        }
      }
    }
  } else {
    // Braintrust run table
    const header = `| ${"Eval".padEnd(
      25
    )} | Result     | Build | Lint  | Tests | Braintrust Link     |`;
    const separator = `|${"-".repeat(
      27
    )}|------------|-------|-------|-------|---------------------|`;

    console.log(header);
    console.log(separator);

    for (const result of results) {
      if (result.status === "fulfilled" && result.value.status === "success") {
        const { evalPath } = result.value;
        const name = evalPath.padEnd(25);
        const result_status = "✅ PASS";
        // For Braintrust runs, we don't have individual build/lint/test results in the same way
        // The experiments handle their own evaluation
        console.log(
          `| ${name} | ${result_status.padEnd(
            10
          )} | ✅    | ✅   | ✅   | View in Dashboard   |`
        );
      } else {
        const evalPath =
          result.status === "fulfilled" ? result.value.evalPath : "Unknown";
        const name = evalPath.padEnd(25);
        console.log(
          `| ${name} | ${"❌ FAIL".padEnd(
            10
          )} | ❌    | ❌   | ❌   | -                   |`
        );
      }
    }

    console.log("\n🔗 View all results at: https://www.braintrust.dev/");
  }

  console.log("═".repeat(120));

  // Summary stats
  const totalEvals = results.length;
  let passedEvals = 0;
  let totalModelRuns = 0;
  let passedModelRuns = 0;

  for (const r of results) {
    if (r.status === "fulfilled" && r.value.status === "success") {
      const evalResult = r.value.result;
      if (isDryRun) {
        if (evalResult?.modelResults) {
          // Multi-model results
          let evalPassed = true;
          for (const modelResult of evalResult.modelResults) {
            totalModelRuns++;
            const passed =
              modelResult.result?.evaluationResults?.buildSuccess &&
              modelResult.result?.evaluationResults?.lintSuccess &&
              modelResult.result?.evaluationResults?.testSuccess;
            if (passed) {
              passedModelRuns++;
            } else {
              evalPassed = false;
            }
          }
          if (evalPassed) passedEvals++;
        } else {
          // Single model results
          totalModelRuns++;
          const passed =
            evalResult?.evaluationResults?.buildSuccess &&
            evalResult?.evaluationResults?.lintSuccess &&
            evalResult?.evaluationResults?.testSuccess;
          if (passed) {
            passedEvals++;
            passedModelRuns++;
          }
        }
      } else {
        // For Braintrust runs, assume success if no error
        passedEvals++;
      }
    }
  }

  if (totalModelRuns > totalEvals) {
    console.log(
      `\n📈 Summary: ${passedEvals}/${totalEvals} evals fully passed (${passedModelRuns}/${totalModelRuns} model runs passed)`
    );
  } else {
    console.log(`\n📈 Summary: ${passedEvals}/${totalEvals} evals passed`);
  }
}

class ProcessPool {
  private activeProcesses = new Map<any, string>(); // process -> evalPath
  private queue: Array<{
    evalPath: string;
    resolve: Function;
    reject: Function;
  }> = [];
  private readonly maxProcesses: number;
  private readonly dryRun: boolean;
  private readonly verbose: boolean;
  private readonly debug: boolean;
  private readonly progressTracker: ProgressTracker | null;
  private memoryCheckInterval: NodeJS.Timeout | null = null;

  constructor(
    maxProcesses: number,
    dryRun: boolean,
    verbose: boolean,
    debug: boolean,
    progressTracker: ProgressTracker | null = null
  ) {
    this.maxProcesses = maxProcesses; // Use the user-specified limit directly
    this.dryRun = dryRun;
    this.verbose = verbose;
    this.debug = debug;
    this.progressTracker = progressTracker;
  }

  async runEvals(evalPaths: string[]): Promise<PromiseSettledResult<any>[]> {
    const promises = evalPaths.map((evalPath) => this.runEval(evalPath));
    const results = await Promise.allSettled(promises);
    await this.cleanup();
    return results;
  }

  private async runEval(evalPath: string): Promise<{
    evalPath: string;
    status: string;
    result?: any;
    error?: string;
  }> {
    return new Promise((resolve, reject) => {
      this.queue.push({ evalPath, resolve, reject });
      this.processQueue();
    });
  }

  private processQueue() {
    while (
      this.queue.length > 0 &&
      this.activeProcesses.size < this.maxProcesses
    ) {
      // Check available memory before spawning new process
      const memoryCheck = checkAvailableMemory(500); // Require 500MB free

      if (!memoryCheck.hasEnough) {
        if (this.verbose) {
          console.log(
            `⚠️  Waiting for memory: ${memoryCheck.available}MB available (need 500MB), ${memoryCheck.percentage}% free`
          );
        }

        // Start periodic memory checking if not already running
        if (!this.memoryCheckInterval && this.queue.length > 0) {
          this.memoryCheckInterval = setInterval(() => {
            const recheckMemory = checkAvailableMemory(500);
            if (recheckMemory.hasEnough) {
              if (this.verbose) {
                console.log(
                  `✓ Memory available again: ${recheckMemory.available}MB (${recheckMemory.percentage}% free)`
                );
              }
              clearInterval(this.memoryCheckInterval!);
              this.memoryCheckInterval = null;
              this.processQueue();
            }
          }, 2000); // Check every 2 seconds
        }
        break;
      }

      if (this.verbose) {
        console.log(
          `📊 Memory: ${memoryCheck.available}MB available (${memoryCheck.percentage}% free)`
        );
      }

      const job = this.queue.shift()!;
      this.startProcess(job);
    }
  }

  private startProcess(job: {
    evalPath: string;
    resolve: Function;
    reject: Function;
  }) {
    // Use bun to run the eval directly instead of worker threads
    const child = spawn(
      "bun",
      [
        "cli.ts",
        "--eval",
        job.evalPath,
        ...(this.dryRun ? ["--dry"] : []),
        ...(this.verbose ? ["--verbose"] : []),
        ...(this.debug ? ["--debug"] : []),
        "--all-models" // Always run all models when using process pool
      ],
      {
        stdio: "pipe", // Always pipe to capture EVAL_RESULT
        cwd: process.cwd()
      }
    );

    this.activeProcesses.set(child, job.evalPath);

    let stdout = "";
    let stderr = "";

    child.stdout?.on("data", (data) => {
      const output = data.toString();
      stdout += output;

      // If verbose, show output but filter out EVAL_RESULT line
      if (this.verbose) {
        const lines = output.split("\n");
        const filteredLines = lines.filter(
          (line) => !line.startsWith("EVAL_RESULT:")
        );
        if (filteredLines.some((line) => line.trim())) {
          process.stdout.write(filteredLines.join("\n"));
        }
      }
    });

    child.stderr?.on("data", (data) => {
      const output = data.toString();
      stderr += output;

      // If verbose, show stderr output
      if (this.verbose) {
        process.stderr.write(output);
      }
    });

    child.on("exit", (code, signal) => {
      this.activeProcesses.delete(child);

      // Update progress tracker immediately when process completes
      if (this.progressTracker && !this.verbose) {
        if (code === 0) {
          this.progressTracker.succeed(job.evalPath);
        } else {
          this.progressTracker.fail(job.evalPath);
        }
      }

      if (code === 0) {
        let result = { success: true };

        // Try to parse JSON result from stdout
        if (stdout.trim()) {
          try {
            const lines = stdout.trim().split("\n");
            const lastLine = lines[lines.length - 1];
            if (lastLine.startsWith("EVAL_RESULT:")) {
              result = JSON.parse(lastLine.replace("EVAL_RESULT:", ""));
            } else {
              // Debug: show what we got instead
              console.warn(
                `No EVAL_RESULT found for ${job.evalPath}. Last line: "${lastLine}"`
              );
              console.warn(
                `Full stdout (last 500 chars): "${stdout.slice(-500)}"`
              );
            }
          } catch (error) {
            // If parsing fails, fallback to simple success
            console.warn(
              `Failed to parse eval result for ${job.evalPath}:`,
              error
            );
          }
        } else {
          console.warn(`Empty stdout for ${job.evalPath}`);
        }

        job.resolve({
          evalPath: job.evalPath,
          status: "success",
          result
        });
      } else {
        job.resolve({
          evalPath: job.evalPath,
          status: "error",
          error: signal
            ? `Process killed by signal ${signal}`
            : `Process exited with code ${code}`
        });
      }

      // Process next job in queue
      this.processQueue();
    });

    child.on("error", (error) => {
      this.activeProcesses.delete(child);

      // Update progress tracker immediately on process error
      if (this.progressTracker && !this.verbose) {
        this.progressTracker.fail(job.evalPath);
      }

      job.resolve({
        evalPath: job.evalPath,
        status: "error",
        error: error.message
      });

      // Process next job in queue
      this.processQueue();
    });
  }

  private async cleanup() {
    // Clear memory check interval
    if (this.memoryCheckInterval) {
      clearInterval(this.memoryCheckInterval);
      this.memoryCheckInterval = null;
    }

    const promises = Array.from(this.activeProcesses.keys()).map(
      (child) =>
        new Promise<void>((resolve) => {
          child.kill("SIGTERM");
          child.on("exit", () => resolve());
          // Force kill after 5 seconds if not terminated
          setTimeout(() => {
            child.kill("SIGKILL");
            resolve();
          }, 5000);
        })
    );
    await Promise.all(promises);
    this.activeProcesses.clear();
  }
}

async function main() {
  try {
    // Set global debug mode
    globalDebugMode = values.debug || false;

    if (values.help) {
      showHelp();
      return;
    }

    if (values.create) {
      if (!values.name || !values.prompt) {
        console.error(
          "Error: --name and --prompt are required when creating a new eval"
        );
        process.exit(1);
      }
      await createNewEval(values.name, values.prompt);
      return;
    }

    // Claude Code mode
    if (values["claude-code"]) {
      const apiKey = values["api-key"] || process.env.ANTHROPIC_API_KEY;
      if (!apiKey) {
        console.error(
          "❌ Error: Anthropic API key is required for Claude Code mode."
        );
        console.error(
          "Set ANTHROPIC_API_KEY environment variable or use --api-key option."
        );
        process.exit(1);
      }

      // Helper to check if an eval is an agent eval
      const isAgentEval = (evalPath: string) => /^agent-\d+/.test(evalPath);

      // Determine if we need dev server
      // Auto-enable for agent evals, or if explicitly requested
      const agentEvalsOnly = values["agent-evals"] || false;
      const singleEval = values.eval || positionals[0];
      const isRunningAgentEval =
        agentEvalsOnly || (singleEval && isAgentEval(singleEval));

      // Dev server auto-starts for agent evals (agent-* pattern)
      const withDevServer = isRunningAgentEval;

      // --with-hooks specifies hook name (e.g., "nextjs-mcp" -> nextjs-mcp-pre.sh, nextjs-mcp-post.sh)
      const hooksName = values["with-hooks"];
      const hooks = hooksName
        ? {
            preEval: `scripts/eval-hooks/${hooksName}-pre.sh`,
            postEval: `scripts/eval-hooks/${hooksName}-post.sh`
          }
        : undefined;

      const claudeOptions = {
        verbose: values.verbose || false,
        debug: values.debug || false,
        timeout: values["claude-timeout"]
          ? parseInt(values["claude-timeout"])
          : 600000, // 10 minutes default
        apiKey,
        devServer: withDevServer
          ? {
              enabled: true,
              command: values["dev-server-cmd"] || "npm run dev",
              port: values["dev-server-port"]
                ? parseInt(values["dev-server-port"])
                : 4000
            }
          : undefined,
        hooks,
        visualDiff: values["with-visual-diff"] || false
      };

      if (values.all) {
        // Run all evals with Claude Code
        const agentEvalsOnly = values["agent-evals"] || false;
        const allEvals = await getAllEvals(agentEvalsOnly);
        const evalType = agentEvalsOnly ? "agent evals" : "evals";

        // Use git worktrees for concurrent execution
        const requestedThreads = values.threads ? parseInt(values.threads) : 1;
        const threads = requestedThreads;

        if (threads > 1) {
          console.log(
            `🔀 Using git worktrees for concurrent execution (${threads} at a time)\n`
          );
        }

        console.log(
          `🤖 Running ${allEvals.length} ${evalType} with Claude Code...\n`
        );

        const results: { evalPath: string; result: ClaudeCodeResult }[] = [];
        const startTime = performance.now();

        // Run evals with concurrency limit
        if (threads === 1) {
          // Sequential execution
          for (const evalPath of allEvals) {
            try {
              console.log(` ▶ ${evalPath}`);
              const result = await runClaudeCodeEval(
                evalPath,
                claudeOptions,
                false
              );
              results.push({ evalPath, result });

              const success =
                result.success &&
                result.buildSuccess &&
                result.lintSuccess &&
                result.testSuccess;

              console.log(`   ${success ? "✅ done" : "❌ failed"}`);
            } catch (error) {
              const errorResult: ClaudeCodeResult = {
                success: false,
                output: "",
                error: error instanceof Error ? error.message : String(error),
                duration: 0
              };
              results.push({ evalPath, result: errorResult });
              console.log(`   ❌ failed`);
            }
          }
        } else {
          // Concurrent execution with limit using worktrees
          const runBatch = async (batch: string[]) => {
            return Promise.all(
              batch.map(async (evalPath) => {
                try {
                  console.log(` ▶ ${evalPath}`);
                  const result = await runClaudeCodeEval(
                    evalPath,
                    claudeOptions,
                    true
                  ); // Use worktrees

                  const success =
                    result.success &&
                    result.buildSuccess &&
                    result.lintSuccess &&
                    result.testSuccess;

                  console.log(`   ${success ? "✅ done" : "❌ failed"}`);
                  return { evalPath, result };
                } catch (error) {
                  const errorResult: ClaudeCodeResult = {
                    success: false,
                    output: "",
                    error:
                      error instanceof Error ? error.message : String(error),
                    duration: 0
                  };
                  console.log(`   ❌ failed`);
                  return { evalPath, result: errorResult };
                }
              })
            );
          };

          // Process in batches
          for (let i = 0; i < allEvals.length; i += threads) {
            const batch = allEvals.slice(i, i + threads);
            const batchResults = await runBatch(batch);
            results.push(...batchResults);
          }
        }

        // Display results in table format
        console.log("\n📊 Results:\n" + "═".repeat(80));
        console.log(formatClaudeCodeResultsTable(results));
        console.log("\n📋 Legend: ✅✅✅ = Build/Lint/Test");

        // Display error details if any
        const failedResults = results.filter(
          (r) =>
            !r.result.buildSuccess ||
            !r.result.lintSuccess ||
            !r.result.testSuccess
        );
        if (failedResults.length > 0) {
          console.log("\n❌ Error Details:");
          console.log("─".repeat(80));
          for (const { evalPath, result } of failedResults) {
            console.log(`\n${evalPath}:`);
            if (!result.buildSuccess && result.buildOutput) {
              console.log(`  Build: ${result.buildOutput.substring(0, 200)}`);
            }
            if (!result.lintSuccess && result.lintOutput) {
              console.log(`  Lint: ${result.lintOutput.substring(0, 200)}`);
            }
            if (!result.testSuccess && result.testOutput) {
              console.log(`  Test: ${result.testOutput.substring(0, 200)}`);
            }
            if (result.error) {
              console.log(`  Error: ${result.error}`);
            }
          }
        }

        console.log("═".repeat(80));

        // Display summary with total time
        const passed = results.filter(
          (r) =>
            r.result.success &&
            r.result.buildSuccess &&
            r.result.lintSuccess &&
            r.result.testSuccess
        ).length;
        const wallClockTime = ((performance.now() - startTime) / 1000).toFixed(
          1
        );
        const totalWorkTime = results.reduce(
          (sum, r) => sum + (r.result.duration || 0),
          0
        );
        const totalWorkTimeSec = (totalWorkTime / 1000).toFixed(1);

        if (threads > 1) {
          console.log(
            `\n📈 Summary: ${passed}/${results.length} evals passed (${wallClockTime}s wall-clock, ${totalWorkTimeSec}s combined work)`
          );
        } else {
          console.log(
            `\n📈 Summary: ${passed}/${results.length} evals passed (${wallClockTime}s total)`
          );
        }

        process.exit(passed === results.length ? 0 : 1);
      } else if (values.evals) {
        // Run multiple specific evals with Claude Code
        const evalNames = values.evals.split(",").map((e: string) => e.trim());

        // Validate that all specified evals exist
        // Check both regular and agent evals
        const regularEvals = await getAllEvals(false);
        const agentEvals = await getAllEvals(true);
        const allEvals = [...regularEvals, ...agentEvals];

        const invalidEvals = evalNames.filter(
          (e: string) => !allEvals.includes(e)
        );
        if (invalidEvals.length > 0) {
          console.error(
            `Error: The following evals do not exist: ${invalidEvals.join(", ")}`
          );
          console.log("\nAvailable evals:");
          console.log("\nRegular evals:");
          regularEvals.forEach((evalName) => console.log(`  ${evalName}`));
          console.log("\nAgent evals:");
          agentEvals.forEach((evalName) => console.log(`  ${evalName}`));
          process.exit(1);
        }

        // Determine concurrency using git worktrees
        const requestedThreads = values.threads ? parseInt(values.threads) : 1;
        const threads = requestedThreads;

        if (threads > 1) {
          console.log(
            `🔀 Using git worktrees for concurrent execution (${threads} at a time)\n`
          );
        }
        console.log(`🤖 Running ${evalNames.length} Claude Code evals...\n`);

        const results: Array<{ evalPath: string; result: ClaudeCodeResult }> =
          [];
        const startTime = performance.now();

        if (threads === 1) {
          // Sequential execution
          for (const evalPath of evalNames) {
            try {
              console.log(` ▶ ${evalPath}`);
              const result = await runClaudeCodeEval(
                evalPath,
                claudeOptions,
                false
              );
              results.push({ evalPath, result });

              const success =
                result.success &&
                result.buildSuccess &&
                result.lintSuccess &&
                result.testSuccess;

              console.log(`   ${success ? "✅ done" : "❌ failed"}`);
            } catch (error) {
              const errorResult: ClaudeCodeResult = {
                success: false,
                buildSuccess: false,
                lintSuccess: false,
                testSuccess: false,
                error: error instanceof Error ? error.message : String(error)
              };
              results.push({ evalPath, result: errorResult });
              console.log(`   ❌ failed`);
            }
          }
        } else {
          // Concurrent execution with limit using worktrees
          const runBatch = async (batch: string[]) => {
            return Promise.all(
              batch.map(async (evalPath) => {
                try {
                  console.log(` ▶ ${evalPath}`);
                  const result = await runClaudeCodeEval(
                    evalPath,
                    claudeOptions,
                    true
                  ); // Use worktrees

                  const success =
                    result.success &&
                    result.buildSuccess &&
                    result.lintSuccess &&
                    result.testSuccess;

                  console.log(`   ${success ? "✅ done" : "❌ failed"}`);
                  return { evalPath, result };
                } catch (error) {
                  const errorResult: ClaudeCodeResult = {
                    success: false,
                    buildSuccess: false,
                    lintSuccess: false,
                    testSuccess: false,
                    error:
                      error instanceof Error ? error.message : String(error)
                  };
                  console.log(`   ❌ failed`);
                  return { evalPath, result: errorResult };
                }
              })
            );
          };

          // Process in batches
          for (let i = 0; i < evalNames.length; i += threads) {
            const batch = evalNames.slice(i, i + threads);
            const batchResults = await runBatch(batch);
            results.push(...batchResults);
          }
        }

        // Display results in table format
        console.log("\n📊 Results:\n" + "═".repeat(80));
        console.log(formatClaudeCodeResultsTable(results));
        console.log("\n📋 Legend: ✅✅✅ = Build/Lint/Test");

        // Display error details if any
        const failedResults = results.filter(
          (r) =>
            !r.result.buildSuccess ||
            !r.result.lintSuccess ||
            !r.result.testSuccess
        );
        if (failedResults.length > 0) {
          console.log("\n❌ Error Details:");
          console.log("─".repeat(80));
          for (const { evalPath, result } of failedResults) {
            console.log(`\n${evalPath}:`);
            if (!result.buildSuccess && result.buildOutput) {
              console.log(`  Build: ${result.buildOutput.substring(0, 200)}`);
            }
            if (!result.lintSuccess && result.lintOutput) {
              console.log(`  Lint: ${result.lintOutput.substring(0, 200)}`);
            }
            if (!result.testSuccess && result.testOutput) {
              console.log(`  Test: ${result.testOutput.substring(0, 200)}`);
            }
            if (result.error) {
              console.log(`  Error: ${result.error}`);
            }
          }
        }

        console.log("═".repeat(80));

        // Display summary with total time
        const passed = results.filter(
          (r) =>
            r.result.success &&
            r.result.buildSuccess &&
            r.result.lintSuccess &&
            r.result.testSuccess
        ).length;
        const wallClockTime = ((performance.now() - startTime) / 1000).toFixed(
          1
        );
        const totalWorkTime = results.reduce(
          (sum, r) => sum + (r.result.duration || 0),
          0
        );
        const totalWorkTimeSec = (totalWorkTime / 1000).toFixed(1);

        if (threads > 1) {
          console.log(
            `\n📈 Summary: ${passed}/${results.length} evals passed (${wallClockTime}s wall-clock, ${totalWorkTimeSec}s combined work)`
          );
        } else {
          console.log(
            `\n📈 Summary: ${passed}/${results.length} evals passed (${wallClockTime}s total)`
          );
        }

        process.exit(passed === results.length ? 0 : 1);
      } else {
        // Single eval with Claude Code
        const evalPath = values.eval || positionals[0];
        if (!evalPath) {
          console.error("❌ Error: No eval specified for Claude Code mode.");
          const allEvals = await getAllEvals();
          console.log("\nAvailable evals:");
          allEvals.forEach((evalName) => console.log(`  ${evalName}`));
          process.exit(1);
        }

        console.log(`🤖 Running Claude Code eval: ${evalPath}\n`);

        const result = await runClaudeCodeEval(evalPath, claudeOptions);

        // Format result for display
        const formattedResult = {
          evaluationResults: {
            buildSuccess: result.buildSuccess,
            lintSuccess: result.lintSuccess,
            testSuccess: result.testSuccess,
            buildOutput: result.buildOutput,
            lintOutput: result.lintOutput,
            testOutput: result.testOutput
          }
        };

        // Display results in table format
        await displaySingleResult(
          evalPath,
          formattedResult,
          values.dry ?? false,
          "Claude Code"
        );

        // Display duration
        const durationSec = (result.duration / 1000).toFixed(1);
        console.log(`\n⏱️  Duration: ${durationSec}s`);

        const success =
          result.success &&
          result.buildSuccess &&
          result.lintSuccess &&
          result.testSuccess;
        process.exit(success ? 0 : 1);
      }
    }

    // Handle --evals flag (multiple specific evals)
    if (values.evals) {
      const evalNames = values.evals.split(",").map((e: string) => e.trim());

      // Validate that all specified evals exist
      // Check both regular and agent evals
      const regularEvals = await getAllEvals(false);
      const agentEvals = await getAllEvals(true);
      const allEvals = [...regularEvals, ...agentEvals];

      if (values.verbose) {
        console.log(
          `Found ${regularEvals.length} regular evals, ${agentEvals.length} agent evals`
        );
        console.log(`Requested: ${evalNames.join(", ")}`);
      }

      const invalidEvals = evalNames.filter(
        (e: string) => !allEvals.includes(e)
      );
      if (invalidEvals.length > 0) {
        console.error(
          `Error: The following evals do not exist: ${invalidEvals.join(", ")}`
        );
        console.log("\nAvailable evals:");
        console.log("\nRegular evals:");
        regularEvals.forEach((evalName) => console.log(`  ${evalName}`));
        console.log("\nAgent evals:");
        agentEvals.forEach((evalName) => console.log(`  ${evalName}`));
        process.exit(1);
      }

      // Clean up any leftover output-dry folders before starting
      await cleanup(values.debug);

      // Ensure shared dependencies are installed first
      const { ensureSharedDependencies } = await import("./lib/eval-runner");
      await ensureSharedDependencies(values.verbose || false);

      // Parse threads option
      const maxThreads = values.threads ? parseInt(values.threads) : 1;
      const threadCount = Math.max(1, maxThreads);

      if (values["claude-code"]) {
        // Run multiple evals with Claude Code
        console.log(
          `🤖 Running ${evalNames.length} Claude Code evals (${threadCount} concurrent)...\n`
        );

        const results: Array<{ evalPath: string; result: ClaudeCodeResult }> =
          [];

        if (threadCount === 1) {
          // Sequential execution
          for (const evalPath of evalNames) {
            try {
              console.log(` ▶ ${evalPath}`);
              const result = await runClaudeCodeEval(evalPath, claudeOptions);
              results.push({ evalPath, result });

              const success =
                result.success &&
                result.buildSuccess &&
                result.lintSuccess &&
                result.testSuccess;

              console.log(`   ${success ? "✅ done" : "❌ failed"}`);
            } catch (error) {
              const errorResult: ClaudeCodeResult = {
                success: false,
                buildSuccess: false,
                lintSuccess: false,
                testSuccess: false,
                error: error instanceof Error ? error.message : String(error)
              };
              results.push({ evalPath, result: errorResult });
              console.log(`   ❌ failed`);
            }
          }
        } else {
          // Concurrent execution with limit
          const runBatch = async (batch: string[]) => {
            return Promise.all(
              batch.map(async (evalPath) => {
                try {
                  console.log(` ▶ ${evalPath}`);
                  const result = await runClaudeCodeEval(
                    evalPath,
                    claudeOptions
                  );

                  const success =
                    result.success &&
                    result.buildSuccess &&
                    result.lintSuccess &&
                    result.testSuccess;

                  console.log(`   ${success ? "✅ done" : "❌ failed"}`);
                  return { evalPath, result };
                } catch (error) {
                  const errorResult: ClaudeCodeResult = {
                    success: false,
                    buildSuccess: false,
                    lintSuccess: false,
                    testSuccess: false,
                    error:
                      error instanceof Error ? error.message : String(error)
                  };
                  console.log(`   ❌ failed`);
                  return { evalPath, result: errorResult };
                }
              })
            );
          };

          // Process in batches
          for (let i = 0; i < evalNames.length; i += threadCount) {
            const batch = evalNames.slice(i, i + threadCount);
            const batchResults = await runBatch(batch);
            results.push(...batchResults);
          }
        }

        // Display summary
        console.log("\n📊 Summary:");
        const passed = results.filter(
          (r) =>
            r.result.success &&
            r.result.buildSuccess &&
            r.result.lintSuccess &&
            r.result.testSuccess
        ).length;
        console.log(`Passed: ${passed}/${results.length}`);

        process.exit(passed === results.length ? 0 : 1);
      } else {
        // Run multiple evals with LLM
        if (threadCount > 1) {
          const memInfo = checkAvailableMemory();
          console.log(
            `Running ${evalNames.length} evals using ${threadCount} processes (max ${threadCount} concurrent)...`
          );
          console.log(
            `Memory: ${memInfo.available}MB available / ${memInfo.total}MB total (${memInfo.percentage}% free)\n`
          );

          if (!values.verbose) {
            globalProgressTracker = new ProgressTracker();
            globalProgressTracker.start(evalNames);
          }

          const processPool = new ProcessPool(
            threadCount,
            values.dry || false,
            values.verbose || false,
            values.debug || false
          );

          const results = await processPool.runEvals(evalNames);
          await cleanup(values.debug);
          await displayResultsTable(results, values.dry ?? false);
        } else {
          // Single-threaded execution
          if (!values.verbose) {
            globalProgressTracker = new ProgressTracker();
            globalProgressTracker.start(evalNames);
          }

          const results = await Promise.allSettled(
            evalNames.map(async (evalPath: string) => {
              try {
                const result = await runEval(
                  evalPath,
                  values.dry,
                  values.verbose,
                  values.debug,
                  values["all-models"]
                );

                if (!values.verbose && globalProgressTracker) {
                  globalProgressTracker.succeed(evalPath);
                }

                return { evalPath, result, status: "success" };
              } catch (error) {
                if (!values.verbose && globalProgressTracker) {
                  globalProgressTracker.fail(evalPath);
                }

                if (values.verbose) {
                  console.error(
                    `❌ ${evalPath}: ${
                      error instanceof Error ? error.message : String(error)
                    }`
                  );
                }

                return {
                  evalPath,
                  status: "error",
                  error: error instanceof Error ? error.message : String(error)
                };
              }
            })
          );

          await cleanup(values.debug);

          if (!values.verbose && globalProgressTracker) {
            globalProgressTracker.stop();
          }

          await displayResultsTable(results, values.dry ?? false);
        }
        return;
      }
    }

    if (values.all) {
      // Clean up any leftover output-dry folders before starting
      await cleanup(values.debug);

      // Ensure shared dependencies are installed first
      const { ensureSharedDependencies } = await import("./lib/eval-runner");
      await ensureSharedDependencies(values.verbose || false);

      const allEvals = await getAllEvals();

      // Parse threads option
      const maxThreads = values.threads ? parseInt(values.threads) : 1;
      const threadCount = Math.max(1, maxThreads); // Allow any number of threads, minimum 1

      if (threadCount > 1) {
        const memInfo = checkAvailableMemory();
        console.log(
          `Running ${allEvals.length} evals using ${threadCount} processes (max ${threadCount} concurrent)...`
        );
        console.log(
          `Memory: ${memInfo.available}MB available / ${memInfo.total}MB total (${memInfo.percentage}% free)\n`
        );

        // Initialize progress tracker before creating worker pool
        if (!values.verbose) {
          globalProgressTracker = new ProgressTracker();
          globalProgressTracker.start(allEvals);
        }

        // Use process pool for multi-process execution
        const processPool = new ProcessPool(
          threadCount,
          values.dry || false,
          values.verbose || false,
          values.debug || false,
          globalProgressTracker
        );

        const results = await processPool.runEvals(allEvals);

        // Progress tracker updates happen in real-time in worker message handlers

        // Clean up progress tracker and output folders
        await cleanup(values.debug);

        // Save all results to JSON file
        const outputDir = process.env.OUTPUT_DIR || "/output";
        const allResultsFile = path.join(
          outputDir,
          `eval-results-all-${Date.now()}.json`
        );
        const publicAllResultsFile = path.join(outputDir, `evaluations.json`);
        try {
          await fs.writeFile(
            allResultsFile,
            JSON.stringify(results, null, 2),
            "utf8"
          );
          await fs.writeFile(
            publicAllResultsFile,
            JSON.stringify(results, null, 2),
            "utf8"
          );
          console.log(`📁 All results saved to: ${allResultsFile}`);
        } catch (error) {
          console.warn(`Failed to save all results to file: ${error}`);
        }

        // Display results table
        await displayResultsTable(results, values.dry ?? false);
        return;
      } else {
        // Single-threaded execution (existing logic)
        console.log(`Running ${allEvals.length} evals in parallel...\n`);

        if (!values.verbose) {
          globalProgressTracker = new ProgressTracker();
          globalProgressTracker.start(allEvals);
        }

        const results = await Promise.allSettled(
          allEvals.map(async (evalPath) => {
            try {
              const result = await runEval(
                evalPath,
                values.dry,
                values.verbose,
                values.debug,
                true // all-models flag is true for batch runs to run all models
              );

              if (!values.verbose && globalProgressTracker) {
                globalProgressTracker.succeed(evalPath);
              }

              return { evalPath, status: "success", result };
            } catch (error) {
              if (!values.verbose && globalProgressTracker) {
                globalProgressTracker.fail(evalPath);
              }

              if (values.verbose) {
                console.error(
                  `❌ ${evalPath}: ${
                    error instanceof Error ? error.message : String(error)
                  }`
                );
              }

              return {
                evalPath,
                status: "error",
                error: error instanceof Error ? error.message : String(error)
              };
            }
          })
        );

        // Clean up progress tracker and output folders
        await cleanup(values.debug);

        // Save all results to JSON file
        const outputDir = process.env.OUTPUT_DIR || "/output";
        const allResultsFile = path.join(
          outputDir,
          `eval-results-all-${Date.now()}.json`
        );
        try {
          await fs.writeFile(
            allResultsFile,
            JSON.stringify(results, null, 2),
            "utf8"
          );
          console.log(`📁 All results saved to: ${allResultsFile}`);
        } catch (error) {
          console.warn(`Failed to save all results to file: ${error}`);
        }

        // Display results table
        await displayResultsTable(results, values.dry ?? false);
        return;
      }
    }

    const evalPath = values.eval || positionals[0];
    if (!evalPath) {
      console.error(
        "Error: No eval specified. Use --eval <path>, provide a positional argument, or use --all"
      );
      console.log("\nAvailable evals:");
      const allEvals = await getAllEvals();
      allEvals.forEach((evalName) => console.log(`  ${evalName}`));
      process.exit(1);
    }

    // Clean up any leftover output-dry folders before starting
    await cleanup(values.debug);

    // Ensure shared dependencies are installed first
    const { ensureSharedDependencies } = await import("./lib/eval-runner");
    await ensureSharedDependencies(values.verbose || false);

    let result;
    if (!values.verbose) {
      const spinner = new Spinner();
      spinner.start(`Running ${evalPath}${values.dry ? " (dry run)" : ""}`);

      try {
        result = await runEval(
          evalPath,
          values.dry,
          values.verbose,
          values.debug,
          values["all-models"]
        );
        spinner.succeed(`Completed: ${evalPath}`);
      } catch (error) {
        spinner.fail(`Failed: ${evalPath}`);
        throw error;
      } finally {
        spinner.stop();
        // Clean up after single eval
        await cleanup(values.debug);
      }
    } else {
      console.log(`Running eval: ${evalPath}${values.dry ? " (dry run)" : ""}`);
      try {
        result = await runEval(
          evalPath,
          values.dry,
          values.verbose,
          values.debug,
          values["all-models"]
        );
        console.log(`✅ Completed: ${evalPath}`);
      } finally {
        // Clean up after single eval
        await cleanup(values.debug);
      }
    }

    // Output JSON result for multi-threaded parsing (always needed for process pool)
    const resultJson = JSON.stringify(result);
    console.log(`EVAL_RESULT:${resultJson}`);

    // Display results for single eval after EVAL_RESULT
    await displaySingleResult(
      evalPath,
      result,
      values.dry ?? false,
      MODELS[0]?.name
    );

    // Explicitly exit after single eval completion
    process.exit(0);
  } catch (error) {
    console.error(
      "Error:",
      error instanceof Error ? error.message : String(error)
    );

    // Still output EVAL_RESULT even on error so parent process can handle it
    const errorResult = {
      error: error instanceof Error ? error.message : String(error),
      evaluationResults: {
        buildSuccess: false,
        lintSuccess: false,
        testSuccess: false,
        buildOutput: "",
        lintOutput: "",
        testOutput: error instanceof Error ? error.message : String(error),
        buildDuration: 0,
        lintDuration: 0,
        testDuration: 0
      }
    };
    const errorResultJson = JSON.stringify(errorResult);
    console.log(`EVAL_RESULT:${errorResultJson}`);

    process.exit(1);
  }
}

// @ts-expect-error
if (import.meta.main) {
  main();
}

export { main };
