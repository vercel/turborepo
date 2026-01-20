import fs from "fs/promises";
import path from "path";
import { spawn, ChildProcess } from "child_process";
import { performance } from "perf_hooks";
import { copyFolder, ensureSharedDependencies } from "./eval-runner";

export interface GeminiResult {
  success: boolean;
  output: string;
  error?: string;
  duration: number;
  buildSuccess?: boolean;
  lintSuccess?: boolean;
  testSuccess?: boolean;
  buildOutput?: string;
  lintOutput?: string;
  testOutput?: string;
  evalPath?: string;
  timestamp?: string;
}

export interface GeminiEvalOptions {
  timeout?: number;
  verbose?: boolean;
  debug?: boolean;
  apiKey?: string;
  model?: string;
  outputFile?: string;
  skipFileWrite?: boolean;
}

export class GeminiRunner {
  private processes = new Map<string, ChildProcess>();
  private verbose: boolean;
  private debug: boolean;
  private apiKey?: string;
  private model?: string;

  constructor(options: GeminiEvalOptions = {}) {
    this.verbose = options.verbose || false;
    this.debug = options.debug || false;
    this.apiKey =
      options.apiKey ||
      process.env.GOOGLE_API_KEY ||
      process.env.GEMINI_API_KEY;
    this.model = options.model;
  }

  async runGeminiEval(
    inputDir: string,
    outputDir: string,
    prompt: string,
    timeout: number = 600000 // 10 minutes default
  ): Promise<GeminiResult> {
    const startTime = performance.now();

    try {
      // Ensure output directory exists and copy input files
      await fs.mkdir(outputDir, { recursive: true });
      await copyFolder(inputDir, outputDir, true); // Exclude test files so gemini doesn't see them

      // Ensure shared dependencies are available
      await ensureSharedDependencies(this.verbose);

      if (this.verbose) {
        console.log(`🤖 Running Gemini on ${outputDir}...`);
        console.log(`📝 Prompt: ${prompt}`);
        console.log("─".repeat(80));
      }

      // Run Gemini with the prompt
      const geminiResult = await this.executeGemini(outputDir, prompt, timeout);

      if (!geminiResult.success) {
        return {
          success: false,
          output: geminiResult.output,
          error: geminiResult.error,
          duration: performance.now() - startTime
        };
      }

      // Copy test files and eslint config back for evaluation
      if (this.verbose) {
        console.log(
          "📋 Copying test files and eslint config back for evaluation..."
        );
      }
      await this.copyTestFilesBack(inputDir, outputDir);

      // Run evaluation (build, lint, test) on the modified code
      const evalResults = await this.runEvaluation(outputDir);

      return {
        success: true,
        output: geminiResult.output,
        duration: performance.now() - startTime,
        buildSuccess: evalResults.buildSuccess,
        lintSuccess: evalResults.lintSuccess,
        testSuccess: evalResults.testSuccess,
        buildOutput: evalResults.buildOutput,
        lintOutput: evalResults.lintOutput,
        testOutput: evalResults.testOutput
      };
    } catch (error) {
      return {
        success: false,
        output: "",
        error: error instanceof Error ? error.message : String(error),
        duration: performance.now() - startTime
      };
    } finally {
      // Clean up if not in debug mode
      console.log(`🧹 Cleanup: debug=${this.debug}, outputDir=${outputDir}`);
      if (!this.debug) {
        console.log(`🗑️  Removing output directory...`);
        try {
          await fs.rm(outputDir, { recursive: true, force: true });
          console.log(`✅ Output directory removed`);
        } catch (error) {
          console.log(`⚠️  Cleanup error: ${error}`);
          // Ignore cleanup errors
        }
      } else {
        console.log(`🔍 Debug mode: preserving output directory`);
      }
    }
  }

  private async executeGemini(
    projectDir: string,
    prompt: string,
    timeout: number
  ): Promise<{ success: boolean; output: string; error?: string }> {
    return new Promise((resolve, reject) => {
      const processId = Math.random().toString(36).substr(2, 9);
      const startTime = Date.now();

      // Append instructions to prompt to prevent running dev servers
      const enhancedPrompt = `${prompt}

IMPORTANT: Do not run any pnpm, npm, or yarn commands (like pnpm dev, npm run dev, pnpm install, etc.). Do not start any development servers. Just make the necessary code changes to the files and exit when done. DO Not ask any followup questions either.`;

      // Prepare environment variables
      const env = { ...process.env };
      if (this.apiKey) {
        env.GOOGLE_API_KEY = this.apiKey;
        env.GEMINI_API_KEY = this.apiKey;
      }

      // Spawn Gemini process with appropriate flags for non-interactive mode
      // -y/--yolo: auto-accept all actions without prompts
      // --include-directories: Allow gemini to work in this directory
      // We'll pass the prompt via stdin to avoid escaping issues
      const args = [
        "-y", // YOLO mode - auto-accept all actions
        "--include-directories",
        projectDir // Allow gemini to work in the project directory
      ];

      // Add model flag if specified
      if (this.model) {
        args.push("-m", this.model);
      }

      console.log("🚀 Spawning gemini process with:");
      console.log("  Command: gemini");
      console.log("  Args:", args);
      console.log("  Working Directory:", projectDir);
      console.log("  API Key present:", !!this.apiKey);
      if (this.model) {
        console.log("  Model:", this.model);
      }
      console.log("  Prompt length:", enhancedPrompt.length, "chars");

      const geminiProcess = spawn("gemini", args, {
        cwd: projectDir,
        env,
        stdio: ["pipe", "pipe", "pipe"] // Use pipe for stdin to send the prompt
      });
      this.processes.set(processId, geminiProcess);

      // Send the enhanced prompt via stdin and close it
      if (geminiProcess.stdin) {
        geminiProcess.stdin.write(enhancedPrompt);
        geminiProcess.stdin.end();
      }

      let stdout = "";
      let stderr = "";
      let lastOutputTime = startTime;
      let resolved = false;

      const idleTimeoutMs = 90000; // 90 second idle timeout
      let idleTimeoutHandle: NodeJS.Timeout | null = null;

      function resolveOnce(result: {
        success: boolean;
        output: string;
        error?: string;
      }) {
        if (resolved) return;
        resolved = true;
        clearTimeout(absoluteTimeoutId);
        if (idleTimeoutHandle) clearTimeout(idleTimeoutHandle);
        clearInterval(heartbeat);
        resolve(result);
      }

      function resetIdleTimeout() {
        if (idleTimeoutHandle) clearTimeout(idleTimeoutHandle);

        idleTimeoutHandle = setTimeout(() => {
          const sinceLastOutput = Date.now() - lastOutputTime;
          console.log(
            `⏱️  Idle timeout reached (${(sinceLastOutput / 1000).toFixed(1)}s since last output)`
          );
          console.log(
            `🛑 Forcefully terminating gemini process ${geminiProcess.pid}...`
          );
          geminiProcess.kill("SIGTERM");

          setTimeout(() => {
            if (!resolved) {
              console.log(
                `🛑 Process didn't respond to SIGTERM, using SIGKILL...`
              );
              geminiProcess.kill("SIGKILL");
            }
          }, 5000);
        }, idleTimeoutMs);
      }

      // Start idle timeout
      resetIdleTimeout();

      // Set up a heartbeat to show the process is still running
      const heartbeat = setInterval(() => {
        const elapsed = Date.now() - startTime;
        const sinceLastOutput = Date.now() - lastOutputTime;
        console.log(
          `⏳ Gemini still running... (${(elapsed / 1000).toFixed(1)}s elapsed, ${(sinceLastOutput / 1000).toFixed(1)}s since last output)`
        );
      }, 5000); // Log every 5 seconds

      geminiProcess.stdout?.on("data", (data) => {
        const output = data.toString();
        lastOutputTime = Date.now();
        resetIdleTimeout();
        // Always log stdout in real-time to help debug
        process.stdout.write(`[gemini stdout] ${output}`);
        // Also log raw bytes to see if there are hidden characters
        if (this.verbose) {
          console.log(`[DEBUG] stdout bytes: ${JSON.stringify(output)}`);
        }
        stdout += output;
      });

      geminiProcess.stderr?.on("data", (data) => {
        const output = data.toString();
        lastOutputTime = Date.now();
        resetIdleTimeout();
        // Always log stderr in real-time to help debug
        process.stderr.write(`[gemini stderr] ${output}`);
        if (this.verbose) {
          console.log(`[DEBUG] stderr bytes: ${JSON.stringify(output)}`);
        }
        stderr += output;
      });

      const absoluteTimeoutId = setTimeout(() => {
        console.log(`⏱️  Absolute timeout reached (${timeout}ms)`);
        geminiProcess.kill("SIGTERM");
        setTimeout(() => {
          geminiProcess.kill("SIGKILL");
        }, 5000);
        resolveOnce({
          success: false,
          output: stdout,
          error: `Gemini process timed out after ${timeout}ms`
        });
      }, timeout);

      geminiProcess.on("exit", (code, signal) => {
        const elapsed = Date.now() - startTime;
        console.log(
          `✓ Gemini process exited with code: ${code}, signal: ${signal} after ${(elapsed / 1000).toFixed(1)}s`
        );

        resolveOnce({
          success: code === 0 && !signal,
          output: stdout,
          error: signal
            ? `Gemini process killed by signal ${signal}`
            : code !== 0
              ? stderr || `Gemini process exited with code ${code}`
              : undefined
        });
      });

      geminiProcess.on("error", (error) => {
        console.log(`❌ Gemini process error: ${error.message}`);
        resolveOnce({
          success: false,
          output: stdout,
          error: error.message
        });
      });

      console.log(`📍 Gemini process spawned with PID: ${geminiProcess.pid}`);
    });
  }

  private async copyTestFilesBack(
    inputDir: string,
    outputDir: string
  ): Promise<void> {
    const entries = await fs.readdir(inputDir, { withFileTypes: true });

    for (const entry of entries) {
      if (entry.name === "node_modules") {
        continue;
      }

      const isTestFile =
        entry.name.endsWith(".test.tsx") ||
        entry.name.endsWith(".test.ts") ||
        entry.name.endsWith(".spec.tsx") ||
        entry.name.endsWith(".spec.ts") ||
        entry.name.endsWith(".test.jsx") ||
        entry.name.endsWith(".test.js") ||
        entry.name.endsWith(".spec.jsx") ||
        entry.name.endsWith(".spec.js");
      const isTestDir =
        entry.name === "__tests__" ||
        entry.name === "test" ||
        entry.name === "tests";
      const isEslintConfig =
        entry.name === ".eslintrc.json" ||
        entry.name === ".eslintrc.js" ||
        entry.name === ".eslintrc.cjs" ||
        entry.name === ".eslintrc.yml" ||
        entry.name === ".eslintrc.yaml" ||
        entry.name === "eslint.config.js" ||
        entry.name === "eslint.config.mjs" ||
        entry.name === "eslint.config.cjs";

      const srcPath = path.join(inputDir, entry.name);
      const destPath = path.join(outputDir, entry.name);

      try {
        if (isTestFile || isEslintConfig) {
          // Copy the test file or eslint config
          await fs.copyFile(srcPath, destPath);
        } else if (entry.isDirectory() && isTestDir) {
          // Copy the test directory
          await copyFolder(srcPath, destPath, false); // Don't exclude anything when copying test dirs
        } else if (entry.isDirectory()) {
          // Recursively copy test files from subdirectories
          await this.copyTestFilesBack(srcPath, destPath);
        }
      } catch (error) {
        // Ignore errors (e.g., directory doesn't exist in output)
      }
    }
  }

  private async runEvaluation(projectDir: string): Promise<{
    buildSuccess: boolean;
    lintSuccess: boolean;
    testSuccess: boolean;
    buildOutput: string;
    lintOutput: string;
    testOutput: string;
  }> {
    let buildSuccess = false;
    let buildOutput = "";
    let lintSuccess = false;
    let lintOutput = "";
    let testSuccess = false;
    let testOutput = "";

    // Run next build
    try {
      if (this.verbose) {
        console.log("Running build...");
      }
      buildOutput = await this.execCommand(
        `cd "${projectDir}" && ../../node_modules/.bin/next build`,
        60000
      );
      buildSuccess = true;
      if (this.verbose) {
        console.log("✅ Build completed");
      }
    } catch (error) {
      if (error && typeof error === "object" && "stdout" in error) {
        buildOutput += (error as any).stdout || "";
        if ((error as any).stderr) {
          buildOutput += "\n" + (error as any).stderr;
        }
      } else {
        buildOutput += error instanceof Error ? error.message : String(error);
      }
      if (this.verbose) {
        console.log("❌ Build failed");
      }
    }

    // Run linting
    try {
      if (this.verbose) {
        console.log("Running lint...");
      }

      // Check if .eslintrc.json exists, create a basic one if not
      const eslintConfigPath = path.join(projectDir, ".eslintrc.json");
      const eslintConfigExists = await fs
        .stat(eslintConfigPath)
        .then(() => true)
        .catch(() => false);

      if (!eslintConfigExists) {
        const basicEslintConfig = {
          extends: "next/core-web-vitals"
        };
        await fs.writeFile(
          eslintConfigPath,
          JSON.stringify(basicEslintConfig, null, 2)
        );
      }

      lintOutput = await this.execCommand(
        `cd "${projectDir}" && ../../node_modules/.bin/next lint`,
        30000
      );
      lintSuccess = true;
      if (this.verbose) {
        console.log("✅ Lint completed");
      }
    } catch (error) {
      if (error && typeof error === "object" && "stdout" in error) {
        lintOutput = (error as any).stdout || "";
        if ((error as any).stderr) {
          lintOutput += "\n" + (error as any).stderr;
        }
      } else {
        lintOutput = error instanceof Error ? error.message : String(error);
      }
      if (this.verbose) {
        console.log("❌ Lint failed");
      }
    }

    // Run tests
    try {
      if (this.verbose) {
        console.log("Running tests...");
      }
      testOutput = await this.execCommand(
        `cd "${projectDir}" && ../../node_modules/.bin/vitest run`,
        30000
      );
      testSuccess = true;
      if (this.verbose) {
        console.log("✅ Tests completed");
      }
    } catch (error) {
      if (error && typeof error === "object" && "stdout" in error) {
        testOutput = (error as any).stdout || "";
        if ((error as any).stderr) {
          testOutput += "\n" + (error as any).stderr;
        }
      } else {
        testOutput = error instanceof Error ? error.message : String(error);
      }
      if (this.verbose) {
        console.log("❌ Tests failed");
      }
    }

    return {
      buildSuccess,
      buildOutput,
      lintSuccess,
      lintOutput,
      testSuccess,
      testOutput
    };
  }

  private async execCommand(command: string, timeout: number): Promise<string> {
    return new Promise((resolve, reject) => {
      const { exec } = require("child_process");
      const process = exec(
        command,
        {
          maxBuffer: 10 * 1024 * 1024, // 10MB buffer
          timeout
        },
        (error: any, stdout: string, stderr: string) => {
          if (error) {
            error.stdout = stdout;
            error.stderr = stderr;
            reject(error);
          } else {
            resolve(stdout);
          }
        }
      );
    });
  }

  async cleanup(): Promise<void> {
    const promises = Array.from(this.processes.entries()).map(
      ([processId, process]) =>
        new Promise<void>((resolve) => {
          process.kill("SIGTERM");
          process.on("exit", () => {
            this.processes.delete(processId);
            resolve();
          });
          // Force kill after 5 seconds if not terminated
          setTimeout(() => {
            process.kill("SIGKILL");
            this.processes.delete(processId);
            resolve();
          }, 5000);
        })
    );
    await Promise.all(promises);
  }
}

export async function runGeminiEval(
  evalPath: string,
  options: GeminiEvalOptions = {}
): Promise<GeminiResult> {
  const evalsDir = path.join(process.cwd(), "evals");
  const fullEvalPath = path.join(evalsDir, evalPath);

  // Check if the eval directory exists
  const evalStat = await fs.stat(fullEvalPath).catch(() => null);
  if (!evalStat || !evalStat.isDirectory()) {
    throw new Error(`Eval directory not found: ${evalPath}`);
  }

  // Look for input directory
  const inputDir = path.join(fullEvalPath, "input");
  const inputExists = await fs
    .stat(inputDir)
    .then((s) => s.isDirectory())
    .catch(() => false);
  if (!inputExists) {
    throw new Error(`No input directory found in ${evalPath}`);
  }

  // Read prompt from prompt.md
  const promptFile = path.join(fullEvalPath, "prompt.md");
  const promptExists = await fs
    .stat(promptFile)
    .then((s) => s.isFile())
    .catch(() => false);
  if (!promptExists) {
    throw new Error(`No prompt.md file found in ${evalPath}`);
  }

  const prompt = await fs.readFile(promptFile, "utf8");
  const outputDir = path.join(fullEvalPath, "output-gemini");

  const runner = new GeminiRunner(options);

  try {
    const result = await runner.runGeminiEval(
      inputDir,
      outputDir,
      prompt,
      options.timeout
    );

    // Add evalPath and timestamp to result
    const timestamp = new Date().toISOString();
    const enrichedResult: GeminiResult = {
      ...result,
      evalPath,
      timestamp
    };

    // Write results to file unless skipFileWrite is true
    if (!options.skipFileWrite) {
      // Determine output file path
      let outputFile = options.outputFile;
      if (!outputFile) {
        // Create default output file in results directory
        const resultsDir = path.join(process.cwd(), "results");
        await fs.mkdir(resultsDir, { recursive: true });
        const sanitizedEvalPath = evalPath.replace(/\//g, "-");
        const timestampStr = Date.now();
        outputFile = path.join(
          resultsDir,
          `gemini-${sanitizedEvalPath}-${timestampStr}.json`
        );
      }

      // Write results to file
      try {
        await fs.writeFile(
          outputFile,
          JSON.stringify(enrichedResult, null, 2),
          "utf-8"
        );
        console.log(`📝 Results written to: ${outputFile}`);
      } catch (error) {
        console.error(
          `⚠️  Failed to write results to file: ${
            error instanceof Error ? error.message : String(error)
          }`
        );
      }
    }

    return enrichedResult;
  } finally {
    await runner.cleanup();
  }
}
