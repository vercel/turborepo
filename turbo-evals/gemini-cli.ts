#!/usr/bin/env bun

import fs from "fs/promises";
import path from "path";
import { parseArgs } from "util";
import { runGeminiEval, GeminiResult } from "./lib/gemini-runner";

const { values, positionals } = parseArgs({
  args: process.argv.slice(2),
  options: {
    help: { type: "boolean", short: "h" },
    eval: { type: "string", short: "e" },
    all: { type: "boolean", short: "a" },
    verbose: { type: "boolean", short: "v" },
    debug: { type: "boolean" },
    dry: { type: "boolean", short: "d" },
    timeout: { type: "string", short: "t" },
    "api-key": { type: "string" },
    model: { type: "string", short: "m" },
    "output-file": { type: "string" }
  },
  allowPositionals: true
});

function showHelp() {
  console.log(`
Gemini Evals CLI

Usage:
  gemini-cli.ts [options] [eval-path]

Options:
  -h, --help                  Show this help message
  -e, --eval <path>           Run a specific eval by path
  -a, --all                   Run all evals with Gemini
  -v, --verbose               Show detailed logs during eval execution
      --debug                 Persist output folders for debugging (don't clean up)
  -d, --dry                   Run eval locally and preserve output folder (don't clean up)
  -t, --timeout <ms>          Timeout in milliseconds (default: 600000 = 10 minutes)
      --api-key <key>         Google API key (or use GOOGLE_API_KEY/GEMINI_API_KEY env var)
  -m, --model <model>         Model to use (e.g., gemini-2.0-flash, gemini-2.5-flash-lite)
      --output-file <path>    Custom path for results file (default: results/gemini-*.json)

Results are automatically written to the results/ directory.

Examples:
  # Run a specific eval (results auto-saved to results/gemini-*.json)
  bun gemini-cli.ts --eval 001-server-component

  # Run eval by positional argument
  bun gemini-cli.ts 001-server-component

  # Run with verbose output and custom timeout
  bun gemini-cli.ts --eval 001-server-component --verbose --timeout 600000

  # Run with specific model
  bun gemini-cli.ts --eval 001-server-component --model gemini-2.0-flash

  # Run all evals (results auto-saved to results/gemini-all-*.json)
  bun gemini-cli.ts --all

  # Debug mode - keep output folders for inspection
  bun gemini-cli.ts --eval 001-server-component --debug

  # Write results to custom location
  bun gemini-cli.ts --eval 001-server-component --output-file my-results.json
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

function displayResult(evalPath: string, result: GeminiResult) {
  console.log("\n📊 Gemini Results:");
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
      console.log(`Gemini Error: ${result.error}`);
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
  results: { evalPath: string; result: GeminiResult }[]
) {
  const totalTests = results.length;
  console.log(`\n📊 Gemini Results Summary (${totalTests} Tests):`);
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
    geminiError?: string;
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
        errors.geminiError = result.error;
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

      if (failed.geminiError) {
        console.log(`  Gemini: ${failed.geminiError}`);
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

  // Check for API key
  const apiKey =
    values["api-key"] ||
    process.env.GOOGLE_API_KEY ||
    process.env.GEMINI_API_KEY;
  if (!apiKey) {
    console.error("❌ Error: Google API key is required.");
    console.error(
      "Set GOOGLE_API_KEY or GEMINI_API_KEY environment variable or use --api-key option."
    );
    process.exit(1);
  }

  const evalOptions = {
    verbose: values.verbose || false,
    debug: values.debug || false,
    dry: values.dry || false,
    timeout: values.timeout ? parseInt(values.timeout) : 600000, // 10 minutes default
    apiKey,
    model: values.model,
    outputFile: values["output-file"]
  };

  if (values.all) {
    const allEvals = await getAllEvals();
    console.log(
      `Running ${allEvals.length} evals with Gemini...${values.model ? ` (model: ${values.model})` : ""}\n`
    );

    const results: { evalPath: string; result: GeminiResult }[] = [];

    // Don't write individual files - we'll write all results to one file at the end
    const individualEvalOptions = { ...evalOptions, skipFileWrite: true };

    for (const evalPath of allEvals) {
      try {
        console.log(`🚀 Running ${evalPath}...`);
        const result = await runGeminiEval(evalPath, individualEvalOptions);
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
        const errorResult: GeminiResult = {
          success: false,
          output: "",
          error: error instanceof Error ? error.message : String(error),
          duration: 0
        };
        results.push({ evalPath, result: errorResult });
        console.log(`❌ FAIL ${evalPath} - ${errorResult.error}`);
      }
    }

    displayResultsTable(results);

    // Determine output file for all results
    let allResultsFile = evalOptions.outputFile;
    if (!allResultsFile) {
      // Create default output file in results directory
      const resultsDir = path.join(process.cwd(), "results");
      await fs.mkdir(resultsDir, { recursive: true });
      const timestamp = Date.now();
      allResultsFile = path.join(resultsDir, `gemini-all-${timestamp}.json`);
    }

    // Write all results to file
    try {
      await fs.writeFile(
        allResultsFile,
        JSON.stringify(results, null, 2),
        "utf-8"
      );
      console.log(`\n📝 All results written to: ${allResultsFile}`);
    } catch (error) {
      console.error(
        `⚠️  Failed to write results to file: ${
          error instanceof Error ? error.message : String(error)
        }`
      );
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

  console.log(
    `🚀 Running Gemini eval: ${evalPath}${values.model ? ` (model: ${values.model})` : ""}`
  );

  try {
    const result = await runGeminiEval(evalPath, evalOptions);
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
