import fs from "fs/promises";
import { existsSync } from "fs";
import path from "path";
import { spawn, ChildProcess } from "child_process";
import { performance } from "perf_hooks";
import { copyFolder, ensureSharedDependencies } from "./eval-runner";

// Global port allocator for concurrent eval runs
let nextAvailablePort = 4000;
const portLock: { [key: number]: boolean } = {};

export interface ClaudeCodeResult {
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
  visualDiff?: {
    success: boolean;
    screenshotPath?: string;
    pixelDifference?: number;
    error?: string;
  };
  evalPath?: string;
  timestamp?: string;
}

export interface ClaudeCodeEvalOptions {
  timeout?: number;
  verbose?: boolean;
  debug?: boolean;
  apiKey?: string;
  devServer?: {
    enabled: boolean;
    command?: string;
    port?: number;
  };
  hooks?: {
    preEval?: string;
    postEval?: string;
  };
  visualDiff?: boolean;
  outputFormat?: string;
  outputFile?: string;
}

export class ClaudeCodeRunner {
  private processes = new Map<string, ChildProcess>();
  private devServerProcess?: ChildProcess;
  private verbose: boolean;
  private debug: boolean;
  private apiKey?: string;
  private devServer?: { enabled: boolean; command?: string; port?: number };
  private hooks?: { preEval?: string; postEval?: string };
  private visualDiff: boolean;

  constructor(options: ClaudeCodeEvalOptions = {}) {
    this.verbose = options.verbose || false;
    this.debug = options.debug || false;
    this.apiKey = options.apiKey || process.env.ANTHROPIC_API_KEY;
    this.devServer = options.devServer;
    this.hooks = options.hooks;
    this.visualDiff = options.visualDiff || false;
  }

  async runClaudeCodeEval(
    inputDir: string,
    outputDir: string,
    prompt: string,
    evalName: string,
    timeout: number = 600000 // 10 minutes default
  ): Promise<ClaudeCodeResult> {
    const startTime = performance.now();
    let postEvalHookRan = false;

    try {
      // Ensure output directory exists and copy input files
      await fs.mkdir(outputDir, { recursive: true });
      await copyFolder(inputDir, outputDir);

      // If we're in a worktree, install dependencies in outputDir
      if (outputDir.includes(".worktrees/")) {
        if (this.verbose) {
          console.log(`📦 Installing dependencies in worktree...`);
        }

        try {
          const { spawn } = await import("child_process");
          await new Promise<void>((resolve, reject) => {
            const proc = spawn("npm", ["install"], {
              cwd: outputDir,
              stdio: this.verbose ? "inherit" : "pipe"
            });

            proc.on("exit", (code) => {
              if (code === 0) {
                if (this.verbose) {
                  console.log(`✅ Dependencies installed in worktree`);
                }
                resolve();
              } else {
                reject(new Error(`npm install failed with code ${code}`));
              }
            });

            proc.on("error", reject);
          });
        } catch (installError) {
          console.error(`⚠️  Failed to install dependencies: ${installError}`);
          throw installError;
        }
      }

      // Ensure shared dependencies are available
      await ensureSharedDependencies(this.verbose);

      // Start dev server if enabled
      if (this.devServer?.enabled) {
        await this.startDevServer(outputDir, evalName);
      }

      // Run pre-eval hook
      if (this.hooks?.preEval) {
        await this.runHookScript(this.hooks.preEval, outputDir, evalName);
      }

      // Show progress indicator
      process.stdout.write(`🤖 Running Claude Code...`);

      if (this.verbose) {
        console.log(`\n🤖 Running Claude Code on ${outputDir}...`);
        console.log(`📝 Prompt: ${prompt}`);
        console.log("─".repeat(80));
      }

      // Run Claude Code with the prompt
      const claudeResult = await this.executeClaudeCode(
        outputDir,
        prompt,
        timeout
      );

      // Clear progress indicator
      if (!this.verbose) {
        process.stdout.write(`\r🤖 Running Claude Code... ✅\n`);
      }

      if (!claudeResult.success) {
        return {
          success: false,
          output: claudeResult.output,
          error: claudeResult.error,
          duration: performance.now() - startTime
        };
      }

      // Run evaluation (build, lint, test) on the modified code
      const evalResults = await this.runEvaluation(outputDir);

      // Run post-eval hook
      if (this.hooks?.postEval) {
        await this.runHookScript(this.hooks.postEval, outputDir, evalName);
        postEvalHookRan = true;
      }

      return {
        success: true,
        output: claudeResult.output,
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
      // Run post-eval hook even on error (if it hasn't run yet)
      if (this.hooks?.postEval && !postEvalHookRan) {
        try {
          await this.runHookScript(this.hooks.postEval, outputDir, evalName);
        } catch (hookError) {
          // Log but don't fail if post-eval hook fails
          console.error(`Post-eval hook failed: ${hookError}`);
        }
      }
      // Clean up if not in debug mode
      if (!this.debug) {
        try {
          await fs.rm(outputDir, { recursive: true, force: true });
        } catch (error) {
          // Ignore cleanup errors
        }
      }
    }
  }

  private async executeClaudeCode(
    projectDir: string,
    prompt: string,
    timeout: number
  ): Promise<{ success: boolean; output: string; error?: string }> {
    return new Promise((resolve, reject) => {
      const processId = Math.random().toString(36).substr(2, 9);

      // Prepare environment variables
      const env = { ...process.env };
      if (this.apiKey) {
        env.ANTHROPIC_API_KEY = this.apiKey;
      }

      // Enhance the prompt with additional instructions (similar to cursor-agent)
      const enhancedPrompt = `${prompt}

IMPORTANT: Do not run npm, pnpm, yarn, or any package manager commands. Dependencies have already been installed. Do not run build, test, or dev server commands. Just write the code files. DO Not ask any followup questions either.`;

      // Spawn Claude Code process with --print flag for non-interactive mode
      // Additional flags to ensure it works well in automation:
      // --dangerously-skip-permissions: bypass file/execution permission prompts
      // --print: non-interactive mode that prints response and exits
      // --mcp-config: load MCP servers from .mcp.json if it exists
      const mcpConfigPath = path.join(projectDir, ".mcp.json");
      const mcpConfigExists = existsSync(mcpConfigPath);

      const args = [
        ...(mcpConfigExists ? ["--mcp-config", mcpConfigPath] : []),
        "--print",
        "--dangerously-skip-permissions",
        enhancedPrompt
      ];

      if (this.verbose) {
        console.log("🚀 Spawning claude process with:");
        console.log("  Command: claude");
        console.log("  Args:", args);
        console.log("  Working Directory:", projectDir);
        console.log("  API Key present:", !!this.apiKey);
      }

      const claudeProcess = spawn("claude", args, {
        cwd: projectDir,
        env,
        stdio: ["pipe", "pipe", "pipe"] // pipe stdin to send "yes" for MCP prompts
      });
      this.processes.set(processId, claudeProcess);

      // Auto-approve MCP server trust prompt by sending "1" (Yes, proceed)
      if (claudeProcess.stdin) {
        claudeProcess.stdin.write("1\n");
        claudeProcess.stdin.end();
      }

      let stdout = "";
      let stderr = "";

      claudeProcess.stdout?.on("data", (data) => {
        const output = data.toString();
        if (this.verbose) {
          console.log("📝 Claude stdout:", JSON.stringify(output));
        }
        stdout += output;
      });

      claudeProcess.stderr?.on("data", (data) => {
        const output = data.toString();
        if (this.verbose) {
          console.log("⚠️  Claude stderr:", JSON.stringify(output));
        }
        stderr += output;
      });

      const timeoutId = setTimeout(() => {
        claudeProcess.kill("SIGTERM");
        setTimeout(() => {
          claudeProcess.kill("SIGKILL");
        }, 5000);
        resolve({
          success: false,
          output: stdout,
          error: `Claude Code process timed out after ${timeout}ms`
        });
      }, timeout);

      claudeProcess.on("exit", (code, signal) => {
        clearTimeout(timeoutId);
        this.processes.delete(processId);

        if (this.verbose) {
          console.log("─".repeat(80));
          console.log(
            `Claude Code finished with code: ${code}, signal: ${signal}`
          );
        }

        if (signal) {
          resolve({
            success: false,
            output: stdout,
            error: `Claude Code process killed by signal ${signal}`
          });
        } else if (code === 0) {
          resolve({
            success: true,
            output: stdout
          });
        } else {
          resolve({
            success: false,
            output: stdout,
            error: stderr || `Claude Code process exited with code ${code}`
          });
        }
      });

      claudeProcess.on("error", (error) => {
        clearTimeout(timeoutId);
        this.processes.delete(processId);
        resolve({
          success: false,
          output: stdout,
          error: error.message
        });
      });
    });
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

    // Determine node_modules path based on whether we're in a worktree
    // In worktree: ./node_modules (symlinked in outputDir)
    // In regular: ../../node_modules (shared at repo root)
    const nodeModulesPath = projectDir.includes(".worktrees/")
      ? "./node_modules/.bin"
      : "../../node_modules/.bin";

    // Run next build
    try {
      if (this.verbose) {
        console.log("Running build...");
      }
      buildOutput = await this.execCommand(
        `cd "${projectDir}" && ${nodeModulesPath}/next build`,
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

      // Use eslint directly (template includes eslint.config.mjs)
      lintOutput = await this.execCommand(
        `cd "${projectDir}" && ${nodeModulesPath}/eslint .`,
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
        `cd "${projectDir}" && ${nodeModulesPath}/vitest run`,
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

  private async allocatePort(): Promise<number> {
    // Simple synchronized port allocation
    while (portLock[nextAvailablePort]) {
      nextAvailablePort++;
    }
    const port = nextAvailablePort;
    portLock[port] = true;
    nextAvailablePort++;
    return port;
  }

  private releasePort(port: number): void {
    delete portLock[port];
  }

  private async findAvailablePort(startPort: number): Promise<number> {
    const net = await import("net");

    return new Promise((resolve, reject) => {
      const server = net.createServer();

      server.listen(startPort, () => {
        const port = (server.address() as any).port;
        server.close(() => resolve(port));
      });

      server.on("error", (err: any) => {
        if (err.code === "EADDRINUSE") {
          // Port is in use, try next one
          resolve(this.findAvailablePort(startPort + 1));
        } else {
          reject(err);
        }
      });
    });
  }

  private async startDevServer(
    projectDir: string,
    evalName: string
  ): Promise<void> {
    if (!this.devServer?.enabled) return;

    // Only start if not already running
    if (this.devServerProcess) return;

    const command = this.devServer.command || "npm run dev";

    // Allocate a unique port for concurrent execution
    const port = await this.allocatePort();

    // Update the port in devServer config so hooks can use it
    this.devServer.port = port;

    process.stdout.write(
      `🚀 Starting dev server: ${command} on port ${port}...`
    );

    return new Promise((resolve, reject) => {
      const [cmd, ...args] = command.split(" ");

      this.devServerProcess = spawn(cmd, args, {
        cwd: projectDir,
        env: { ...process.env, PORT: String(port) },
        stdio: ["ignore", "pipe", "pipe"]
      });

      let output = "";

      const onData = (data: Buffer) => {
        const str = data.toString();
        output += str;
        if (this.verbose) {
          console.log(`[dev-server] ${str.trim()}`);
        }

        // Check for various "ready" indicators
        if (
          str.includes("Ready in") ||
          str.includes("started server on") ||
          str.includes("Local:") ||
          str.includes(`http://localhost:${port}`)
        ) {
          console.log(` ✅`);
          this.devServerProcess?.stdout?.off("data", onData);
          this.devServerProcess?.stderr?.off("data", onData);
          resolve();
        }
      };

      this.devServerProcess.stdout?.on("data", onData);
      this.devServerProcess.stderr?.on("data", onData);

      this.devServerProcess.on("error", (error) => {
        reject(new Error(`Failed to start dev server: ${error.message}`));
      });

      this.devServerProcess.on("exit", (code) => {
        if (code !== 0) {
          reject(new Error(`Dev server exited with code ${code}\n${output}`));
        }
      });

      // Timeout after 30 seconds
      setTimeout(() => {
        if (this.devServerProcess && !this.devServerProcess.killed) {
          reject(new Error("Dev server startup timeout (30s)\n" + output));
        }
      }, 30000);
    });
  }

  private async stopDevServer(): Promise<void> {
    if (!this.devServerProcess) return;

    const port = this.devServer?.port;

    if (this.verbose) {
      console.log("🛑 Stopping dev server...");
    }

    return new Promise<void>((resolve) => {
      this.devServerProcess!.kill("SIGTERM");
      this.devServerProcess!.on("exit", () => {
        this.devServerProcess = undefined;
        // Release the port back to the pool
        if (port) {
          this.releasePort(port);
        }
        resolve();
      });
      // Force kill after 5 seconds
      setTimeout(() => {
        if (this.devServerProcess && !this.devServerProcess.killed) {
          this.devServerProcess.kill("SIGKILL");
          this.devServerProcess = undefined;
        }
        resolve();
      }, 5000);
    });
  }

  private async runHookScript(
    script: string,
    outputDir: string,
    evalName: string
  ): Promise<void> {
    const port = this.devServer?.port || 3000;
    const evalDir = path.dirname(path.dirname(outputDir)); // Go up from output dir to eval dir

    // Determine if this is pre or post hook based on the script path
    const hookType = script.includes("pre") ? "Pre-eval" : "Post-eval";
    const hookName = path.basename(script);
    process.stdout.write(`🪝 ${hookType} hook: ${hookName}...`);

    return new Promise((resolve, reject) => {
      const hookProcess = spawn("bash", [script], {
        env: {
          ...process.env,
          PORT: String(port),
          OUTPUT_DIR: outputDir,
          EVAL_NAME: evalName,
          EVAL_DIR: evalDir
        },
        stdio: this.verbose ? "inherit" : "pipe"
      });

      hookProcess.on("exit", (code) => {
        if (code === 0) {
          console.log(` ✅`);
          resolve();
        } else {
          console.log(` ❌`);
          reject(new Error(`Hook script exited with code ${code}`));
        }
      });

      hookProcess.on("error", (error) => {
        reject(new Error(`Failed to run hook script: ${error.message}`));
      });
    });
  }

  async cleanup(): Promise<void> {
    // Stop dev server first
    await this.stopDevServer();

    // Then cleanup Claude processes
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

export async function runClaudeCodeEval(
  evalPath: string,
  options: ClaudeCodeEvalOptions = {},
  useWorktree: boolean = false
): Promise<ClaudeCodeResult> {
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

  let outputDir: string;
  let worktreePath: string | undefined;
  let worktreeInputDir: string;

  if (useWorktree) {
    // Create a git worktree for isolated execution
    const worktreesDir = path.join(process.cwd(), ".worktrees");
    await fs.mkdir(worktreesDir, { recursive: true });

    worktreePath = path.join(worktreesDir, `${evalPath}-${Date.now()}`);

    try {
      // Create worktree (detached HEAD to avoid branch conflicts)
      const { spawn } = await import("child_process");
      await new Promise<void>((resolve, reject) => {
        const proc = spawn(
          "git",
          ["worktree", "add", "--detach", worktreePath, "HEAD"],
          {
            cwd: process.cwd(),
            stdio: "pipe"
          }
        );

        proc.on("exit", (code) => {
          if (code === 0) resolve();
          else
            reject(new Error(`Failed to create worktree (exit code ${code})`));
        });

        proc.on("error", reject);
      });

      // We'll symlink node_modules after outputDir is created

      // Also symlink .next build artifacts if they exist
      const mainNextDir = path.join(process.cwd(), ".next");
      const worktreeNextDir = path.join(worktreePath, ".next");
      const nextExists = await fs
        .stat(mainNextDir)
        .then(() => true)
        .catch(() => false);

      if (nextExists) {
        try {
          await fs.symlink(mainNextDir, worktreeNextDir, "dir");
        } catch {
          // Ignore if symlink fails
        }
      }
    } catch (error) {
      throw new Error(`Failed to create worktree: ${error}`);
    }

    // Use flattened paths within the worktree
    // Copy input files directly to worktree root to avoid deep nesting
    worktreeInputDir = inputDir; // Still read from original location
    outputDir = path.join(worktreePath, "output-claude-code");
  } else {
    worktreeInputDir = inputDir;
    outputDir = path.join(fullEvalPath, "output-claude-code");
  }

  const runner = new ClaudeCodeRunner(options);

  try {
    const result = await runner.runClaudeCodeEval(
      worktreeInputDir,
      outputDir,
      prompt,
      evalPath,
      options.timeout
    );
    return result;
  } finally {
    await runner.cleanup();

    // Cleanup worktree if used
    if (worktreePath) {
      try {
        const { spawn } = await import("child_process");
        await new Promise<void>((resolve) => {
          const proc = spawn(
            "git",
            ["worktree", "remove", "--force", worktreePath],
            {
              cwd: process.cwd(),
              stdio: "pipe"
            }
          );

          proc.on("exit", () => resolve());
          proc.on("error", () => resolve()); // Continue even if cleanup fails
        });
      } catch {
        // Ignore cleanup errors
      }
    }
  }
}
