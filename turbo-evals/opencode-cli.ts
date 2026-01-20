#!/usr/bin/env bun

import fs from "fs/promises";
import path from "path";
import { parseArgs } from "util";
import { runOpencodeEval, OpencodeResult } from "./lib/opencode-runner";

const { values, positionals } = parseArgs({
  args: process.argv.slice(2),
  options: {
    help: { type: "boolean", short: "h" },
    eval: { type: "string", short: "e" },
    all: { type: "boolean", short: "a" },
    verbose: { type: "boolean", short: "v" },
    debug: { type: "boolean" },
    timeout: { type: "string", short: "t" },
    model: { type: "string", short: "m" },
    force: { type: "boolean", short: "f" },
    "output-format": { type: "string", short: "o" },
    "output-file": { type: "string" },
    "output-dir": { type: "string", short: "d" }
  },
  allowPositionals: true
});

function showHelp() {
  console.log(`
Opencode Evals CLI

Usage:
  opencode-cli.ts [options] [eval-path]

Options:
  -h, --help                  Show this help message
  -e, --eval <path>           Run a specific eval by path
  -a, --all                   Run all evals with Opencode
  -v, --verbose               Show detailed logs during eval execution
      --debug                 Persist output folders for debugging (don't clean up)
  -t, --timeout <ms>          Timeout in milliseconds (default: 600000 = 10 minutes)
  -m, --model <model>         Model to use (default: opencode/code-supernova)
  -f, --force                 Allow file modifications in automated mode
  -o, --output-format <fmt>   Output format: text, json, or stream-json (default: text)
      --output-file <path>    Write results to JSON file
  -d, --output-dir <path>     Directory to save the output file (default: current directory)

Examples:
  # Run a specific eval
  bun opencode-cli.ts --eval 001-server-component

  # Run eval by positional argument
  bun opencode-cli.ts 001-server-component

  # Run with verbose output and custom timeout
  bun opencode-cli.ts --eval 001-server-component --verbose --timeout 600000

  # Run all evals
  bun opencode-cli.ts --all

  # Run with specific model
  bun opencode-cli.ts --eval 001-server-component --model opencode/grok-code

  # Debug mode - keep output folders for inspection
  bun opencode-cli.ts --eval 001-server-component --debug

  # Write results to JSON file
  bun opencode-cli.ts --eval 001-server-component --output-file results.json

  # Run all evals and save results to specific directory
  bun opencode-cli.ts --all --output-file results.json --output-dir ./my-results
`);
}

async function getAllEvals(): Promise<string[]> {
  const evalsDir = path.join(process.cwd(), "evals");
  const entries = await fs.readdir(evalsDir, { withFileTypes: true });

  const evals: string[] = [];

  for (const entry of entries) {
    if (entry.isDirectory() && /^\d+/.test(entry.name)) {
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

function formatDuration(ms: number): string {
  if (ms < 1000) {
    return `${Math.round(ms)}ms`;
  } else {
    const seconds = ms / 1000;
    return `${seconds.toFixed(1)}s`;
  }
}

function displayResult(evalPath: string, result: OpencodeResult) {
  console.log("\n📊 Opencode Results:");
  console.log("═".repeat(80));

  const evalColWidth = Math.max(25, evalPath.length);
  const header = `| ${"Eval".padEnd(evalColWidth)} | Result     | Build | Lint  | Tests | Duration |`;
  const separator = `|${"-".repeat(evalColWidth + 2)}|------------|-------|-------|-------|----------|`;

  console.log(header);
  console.log(separator);

  const name = evalPath.padEnd(evalColWidth);
  const build = result.buildSuccess ? "✅" : "❌";
  const lint = result.lintSuccess ? "✅" : "❌";
  const tests = result.testSuccess ? "✅" : "❌";
  const allPassed =
    result.buildSuccess && result.lintSuccess && result.testSuccess;
  const resultStatus = allPassed ? "✅ PASS" : "❌ FAIL";
  const duration = formatDuration(result.duration);

  console.log(
    `| ${name} | ${resultStatus.padEnd(10)} | ${build}    | ${lint}   | ${tests}   | ${duration.padEnd(8)} |`
  );

  console.log("═".repeat(80));

  if (!allPassed || !result.success) {
    console.log("\n❌ Error Details:");
    console.log("─".repeat(80));

    if (result.error) {
      console.log(`Opencode Error: ${result.error}`);
    }

    if (!result.buildSuccess && result.buildOutput) {
      console.log(`Build Error:\n${result.buildOutput.slice(-1000)}`);
    }

    if (!result.lintSuccess && result.lintOutput) {
      console.log(`Lint Error:\n${result.lintOutput.slice(-1000)}`);
    }

    if (!result.testSuccess && result.testOutput) {
      console.log(`Test Error:\n${result.testOutput.slice(-1000)}`);
    }
  }

  console.log("═".repeat(80));
}

function displayResultsTable(
  results: { evalPath: string; result: OpencodeResult }[]
) {
  const totalTests = results.length;
  console.log(`\n📊 Opencode Results Summary (${totalTests} Tests):`);
  console.log("═".repeat(120));

  const header = `| ${"Eval".padEnd(25)} | Result     | Build | Lint  | Tests | Duration |`;
  const separator = `|${"-".repeat(27)}|------------|-------|-------|-------|----------|`;

  console.log(header);
  console.log(separator);

  const failedEvals: Array<{
    evalPath: string;
    buildError?: string;
    lintError?: string;
    testError?: string;
    opencodeError?: string;
  }> = [];

  let passedEvals = 0;

  for (const { evalPath, result } of results) {
    const name = evalPath.padEnd(25);
    const build = result.buildSuccess ? "✅" : "❌";
    const lint = result.lintSuccess ? "✅" : "❌";
    const tests = result.testSuccess ? "✅" : "❌";
    const allPassed =
      result.success &&
      result.buildSuccess &&
      result.lintSuccess &&
      result.testSuccess;
    const resultStatus = allPassed ? "✅ PASS" : "❌ FAIL";
    const duration = formatDuration(result.duration);

    if (allPassed) {
      passedEvals++;
    }

    console.log(
      `| ${name} | ${resultStatus.padEnd(10)} | ${build}    | ${lint}   | ${tests}   | ${duration.padEnd(8)} |`
    );

    // Collect errors for failed evals
    if (!allPassed) {
      const errors: any = { evalPath };

      if (result.error) {
        errors.opencodeError = result.error;
      }

      if (!result.buildSuccess && result.buildOutput) {
        errors.buildError = result.buildOutput.slice(-500);
      }

      if (!result.lintSuccess && result.lintOutput) {
        errors.lintError = result.lintOutput.slice(-500);
      }

      if (!result.testSuccess && result.testOutput) {
        errors.testError = result.testOutput.slice(-500);
      }

      failedEvals.push(errors);
    }
  }

  console.log("═".repeat(120));

  // Summary stats
  console.log(`\n📈 Summary: ${passedEvals}/${totalTests} evals passed`);

  // Display error summaries
  if (failedEvals.length > 0) {
    console.log("\n❌ Error Summaries:");
    console.log("─".repeat(120));

    for (const failed of failedEvals) {
      console.log(`\n${failed.evalPath}:`);

      if (failed.opencodeError) {
        console.log(`  Opencode: ${failed.opencodeError}`);
      }

      if (failed.buildError) {
        console.log(`  Build: ${failed.buildError}`);
      }

      if (failed.lintError) {
        console.log(`  Lint: ${failed.lintError}`);
      }

      if (failed.testError) {
        console.log(`  Tests: ${failed.testError}`);
      }
    }
  }
}

async function main() {
  if (values.help) {
    showHelp();
    return;
  }

  const evalOptions = {
    verbose: values.verbose || false,
    debug: values.debug || false,
    timeout: values.timeout ? parseInt(values.timeout) : 600000, // 10 minutes default
    model: values.model || "opencode/code-supernova",
    force: values.force || false,
    outputFormat: values["output-format"] || "text",
    outputFile: values["output-file"],
    outputDir: values["output-dir"]
  };

  if (values.all) {
    const allEvals = await getAllEvals();
    console.log(`Running ${allEvals.length} evals with Opencode...\n`);

    const results: { evalPath: string; result: OpencodeResult }[] = [];

    // Don't pass outputFile to individual runs - we'll write all results at the end
    const individualEvalOptions = { ...evalOptions, outputFile: undefined };

    for (const evalPath of allEvals) {
      try {
        console.log(`🚀 Running ${evalPath}...`);
        const result = await runOpencodeEval(evalPath, individualEvalOptions);
        results.push({ evalPath, result });

        const status =
          result.success &&
          result.buildSuccess &&
          result.lintSuccess &&
          result.testSuccess
            ? "✅ PASS"
            : "❌ FAIL";
        console.log(
          `${status} ${evalPath} (${formatDuration(result.duration)})`
        );
      } catch (error) {
        const errorResult: OpencodeResult = {
          success: false,
          output: "",
          error: error instanceof Error ? error.message : String(error),
          duration: 0,
          buildSuccess: false,
          lintSuccess: false,
          testSuccess: false
        };
        results.push({ evalPath, result: errorResult });
        console.log(`❌ FAIL ${evalPath} - ${errorResult.error}`);
      }
    }

    displayResultsTable(results);

    // Write all results to file if outputFile is specified
    if (evalOptions.outputFile) {
      try {
        // Combine outputDir and outputFile if both are specified
        const outputPath = evalOptions.outputDir
          ? path.join(evalOptions.outputDir, evalOptions.outputFile)
          : evalOptions.outputFile;

        // Create output directory if it doesn't exist
        if (evalOptions.outputDir) {
          await fs.mkdir(evalOptions.outputDir, { recursive: true });
        }

        await fs.writeFile(
          outputPath,
          JSON.stringify(results, null, 2),
          "utf-8"
        );
        console.log(`\n📝 All results written to: ${outputPath}`);
      } catch (error) {
        console.error(
          `⚠️  Failed to write results to file: ${
            error instanceof Error ? error.message : String(error)
          }`
        );
      }
    }

    return;
  }

  const evalPath = values.eval || positionals[0];
  if (!evalPath) {
    console.error(
      "❌ Error: No eval specified. Use --eval <path>, provide a positional argument, or use --all"
    );
    console.log("\nAvailable evals:");
    const allEvals = await getAllEvals();
    allEvals.forEach((evalName) => console.log(`  ${evalName}`));
    process.exit(1);
  }

  console.log(`🚀 Running Opencode eval: ${evalPath}`);

  try {
    const result = await runOpencodeEval(evalPath, evalOptions);
    displayResult(evalPath, result);

    const success =
      result.success &&
      result.buildSuccess &&
      result.lintSuccess &&
      result.testSuccess;
    process.exit(success ? 0 : 1);
  } catch (error) {
    console.error(
      `❌ Error: ${error instanceof Error ? error.message : String(error)}`
    );
    process.exit(1);
  }
}

// @ts-ignore
if (import.meta.main) {
  main().catch((error) => {
    console.error("Unexpected error:", error);
    process.exit(1);
  });
}
