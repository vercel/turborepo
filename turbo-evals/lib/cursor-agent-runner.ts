import fs from "fs/promises";
import path from "path";
import { spawn } from "child_process";
import { tmpdir } from "os";

export interface CursorAgentResult {
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
  streamData?: any[];
  evalPath?: string;
  timestamp?: string;
}

interface CursorAgentOptions {
  verbose?: boolean;
  debug?: boolean;
  timeout?: number;
  apiKey: string;
  force?: boolean;
  outputFormat?: string;
  outputFile?: string;
}

async function execAsync(
  command: string,
  options: {
    cwd?: string;
    timeout?: number;
    env?: Record<string, string>;
    verbose?: boolean;
    idleTimeout?: number;
  } = {}
): Promise<{ stdout: string; stderr: string; exitCode: number }> {
  return new Promise((resolve) => {
    const startTime = Date.now();
    console.log(`[execAsync] Spawning command: ${command}`);
    console.log(`[execAsync] CWD: ${options.cwd}`);
    console.log(`[execAsync] Timeout: ${options.timeout}ms`);
    console.log(`[execAsync] Idle timeout: ${options.idleTimeout || 30000}ms`);

    const child = spawn(command, {
      shell: true,
      cwd: options.cwd,
      env: { ...process.env, ...options.env },
      timeout: options.timeout
    });

    let stdout = "";
    let stderr = "";
    let lastOutputTime = startTime;
    let idleTimeoutHandle: NodeJS.Timeout | null = null;
    let resolved = false;

    const idleTimeoutMs = options.idleTimeout || 30000; // 30 seconds default idle timeout

    function resolveOnce(result: {
      stdout: string;
      stderr: string;
      exitCode: number;
    }) {
      if (resolved) return;
      resolved = true;
      clearInterval(heartbeat);
      if (idleTimeoutHandle) clearTimeout(idleTimeoutHandle);
      resolve(result);
    }

    function resetIdleTimeout() {
      if (idleTimeoutHandle) clearTimeout(idleTimeoutHandle);

      idleTimeoutHandle = setTimeout(() => {
        const sinceLastOutput = Date.now() - lastOutputTime;
        console.log(
          `[execAsync] Idle timeout reached (${(sinceLastOutput / 1000).toFixed(1)}s since last output)`
        );
        console.log(
          `[execAsync] Forcefully terminating process ${child.pid}...`
        );
        child.kill("SIGTERM");

        // If SIGTERM doesn't work after 5 seconds, use SIGKILL
        setTimeout(() => {
          if (!resolved) {
            console.log(
              `[execAsync] Process didn't respond to SIGTERM, using SIGKILL...`
            );
            child.kill("SIGKILL");
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
        `[execAsync] Still running... (${(elapsed / 1000).toFixed(1)}s elapsed, ${(sinceLastOutput / 1000).toFixed(1)}s since last output)`
      );
    }, 5000); // Log every 5 seconds

    child.stdout?.on("data", (data) => {
      const text = data.toString();
      stdout += text;
      lastOutputTime = Date.now();
      resetIdleTimeout(); // Reset the idle timeout on new output
      // Always log stdout in real-time to help debug
      process.stdout.write(`[stdout] ${text}`);
    });

    child.stderr?.on("data", (data) => {
      const text = data.toString();
      stderr += text;
      lastOutputTime = Date.now();
      resetIdleTimeout(); // Reset the idle timeout on new output
      // Always log stderr in real-time to help debug
      process.stderr.write(`[stderr] ${text}`);
    });

    child.on("exit", (code) => {
      const elapsed = Date.now() - startTime;
      console.log(
        `[execAsync] Process exited with code: ${code} after ${(elapsed / 1000).toFixed(1)}s`
      );
      resolveOnce({
        stdout,
        stderr,
        exitCode: code || 0
      });
    });

    child.on("error", (error) => {
      console.log(`[execAsync] Process error: ${error.message}`);
      stderr += error.message;
      resolveOnce({
        stdout,
        stderr,
        exitCode: 1
      });
    });

    console.log(`[execAsync] Process spawned with PID: ${child.pid}`);

    // Also check if stdin needs to be closed (some processes wait for stdin)
    if (child.stdin) {
      console.log(
        `[execAsync] Closing stdin to prevent process from waiting for input...`
      );
      child.stdin.end();
    }
  });
}

async function copyDirectory(
  src: string,
  dest: string,
  excludeTestFiles: boolean = false
) {
  await fs.mkdir(dest, { recursive: true });
  const entries = await fs.readdir(src, { withFileTypes: true });

  for (const entry of entries) {
    if (entry.name === "node_modules") {
      continue;
    }

    // Skip test files and eslint config if requested
    if (excludeTestFiles) {
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

      if (isTestFile || (entry.isDirectory() && isTestDir) || isEslintConfig) {
        continue;
      }
    }

    const srcPath = path.join(src, entry.name);
    const destPath = path.join(dest, entry.name);

    if (entry.isDirectory()) {
      await copyDirectory(srcPath, destPath, excludeTestFiles);
    } else {
      await fs.copyFile(srcPath, destPath);
    }
  }
}

async function copyTestFilesBack(
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
        await copyDirectory(srcPath, destPath, false); // Don't exclude anything when copying test dirs
      } else if (entry.isDirectory()) {
        // Recursively copy test files from subdirectories
        await copyTestFilesBack(srcPath, destPath);
      }
    } catch (error) {
      // Ignore errors (e.g., directory doesn't exist in output)
    }
  }
}

async function installWorkspaceDependencies(
  workspaceDir: string,
  verbose: boolean = false
): Promise<{ success: boolean; output: string }> {
  try {
    const packageJsonPath = path.join(workspaceDir, "package.json");
    const hasPackageJson = await fs
      .stat(packageJsonPath)
      .then(() => true)
      .catch(() => false);

    if (!hasPackageJson) {
      return {
        success: true,
        output: "No package.json found, skipping install"
      };
    }

    if (verbose) {
      console.log("    📦 Running pnpm install...");
    }

    const { stdout, stderr, exitCode } = await execAsync(
      "pnpm install --prefer-offline",
      {
        cwd: workspaceDir,
        timeout: 600000, // 10 minute timeout
        idleTimeout: 60000 // 60 second idle timeout for downloads
      }
    );

    if (exitCode === 0) {
      if (verbose) {
        console.log("    ✓ Dependencies installed");
      }
      return { success: true, output: stdout };
    } else {
      return { success: false, output: stderr || stdout };
    }
  } catch (error) {
    return {
      success: false,
      output: error instanceof Error ? error.message : String(error)
    };
  }
}

export async function runCursorAgentEval(
  evalPath: string,
  options: CursorAgentOptions
): Promise<CursorAgentResult> {
  const startTime = Date.now();
  const verbose = options.verbose || false;

  try {
    console.log(`[1/9] Setting up paths...`);
    // Setup paths
    const evalsDir = path.join(process.cwd(), "evals");
    const evalDir = path.join(evalsDir, evalPath);

    console.log(`[2/9] Verifying eval exists at: ${evalDir}`);
    // Verify eval exists
    const evalExists = await fs
      .stat(evalDir)
      .then(() => true)
      .catch(() => false);

    if (!evalExists) {
      throw new Error(`Eval directory not found: ${evalPath}`);
    }

    console.log(`[3/9] Reading prompt...`);
    // Read prompt
    const promptPath = path.join(evalDir, "prompt.md");
    const prompt = await fs.readFile(promptPath, "utf-8");

    console.log(`[4/9] Creating temporary workspace...`);
    // Create temporary workspace
    const tempDir = path.join(tmpdir(), `cursor-agent-eval-${Date.now()}`);
    const workspaceDir = path.join(tempDir, "workspace");
    await fs.mkdir(workspaceDir, { recursive: true });

    console.log(`[5/9] Copying input files to workspace...`);
    // Copy input files to workspace (exclude test files so cursor doesn't see them)
    const inputDir = path.join(evalDir, "input");
    await copyDirectory(inputDir, workspaceDir, true);

    if (verbose) {
      console.log(`📁 Workspace created at: ${workspaceDir}`);
      console.log(`📝 Prompt: ${prompt.slice(0, 200)}...`);
    }

    console.log(`[6/9] Installing dependencies in workspace...`);
    // Install dependencies in the workspace
    const installResult = await installWorkspaceDependencies(
      workspaceDir,
      verbose
    );
    if (!installResult.success) {
      console.warn(
        `⚠️  Warning: Dependency installation failed: ${installResult.output}`
      );
    }

    console.log(`[7/9] Building cursor-agent command...`);
    // Build the cursor-agent command
    const cursorCommand = buildCursorCommand(workspaceDir, prompt, options);

    console.log(`🔧 Running command: ${cursorCommand}`);

    console.log(
      `[8/9] Executing cursor-agent (timeout: ${options.timeout}ms)...`
    );
    // Execute cursor-agent
    const { stdout, stderr, exitCode } = await execAsync(cursorCommand, {
      cwd: workspaceDir,
      timeout: options.timeout,
      env: {
        CURSOR_API_KEY: options.apiKey
      },
      verbose,
      idleTimeout: 90000 // Kill process if no output for 90 seconds
    });

    console.log(`✅ Cursor agent execution completed (exit code: ${exitCode})`);

    // Parse output based on format
    let streamData: any[] = [];
    if (options.outputFormat === "stream-json") {
      streamData = parseStreamJson(stdout);
    }

    if (verbose) {
      console.log(`📤 Cursor Agent output (${stdout.length} chars)`);
      if (stderr) {
        console.log(`⚠️ Stderr: ${stderr}`);
      }
    }

    console.log(
      `[8.5/9] Copying test files and eslint config back for validation...`
    );
    // Copy test files and eslint config back for validation
    await copyTestFilesBack(inputDir, workspaceDir);

    console.log(`[9/9] Running validation commands...`);
    // Run evaluation commands
    console.log(`  → Running build...`);
    const buildResult = await runBuildCommand(workspaceDir, verbose);
    console.log(`  → Build: ${buildResult.success ? "✅" : "❌"}`);

    console.log(`  → Running lint...`);
    const lintResult = await runLintCommand(workspaceDir, verbose);
    console.log(`  → Lint: ${lintResult.success ? "✅" : "❌"}`);

    console.log(`  → Running tests...`);
    const testResult = await runTestCommand(workspaceDir, verbose);
    console.log(`  → Tests: ${testResult.success ? "✅" : "❌"}`);

    // Clean up temp directory if not in debug mode
    if (!options.debug) {
      console.log(`🧹 Cleaning up workspace...`);
      await fs.rm(tempDir, { recursive: true, force: true });
    } else {
      console.log(`🐛 Debug mode: Workspace preserved at ${workspaceDir}`);
    }

    const duration = Date.now() - startTime;
    console.log(`⏱️  Total duration: ${duration}ms`);

    const result: CursorAgentResult = {
      success: exitCode === 0,
      output: stdout,
      error:
        exitCode !== 0 ? stderr || "Cursor Agent execution failed" : undefined,
      duration,
      buildSuccess: buildResult.success,
      lintSuccess: lintResult.success,
      testSuccess: testResult.success,
      buildOutput: buildResult.output,
      lintOutput: lintResult.output,
      testOutput: testResult.output,
      streamData: streamData.length > 0 ? streamData : undefined,
      evalPath,
      timestamp: new Date().toISOString()
    };

    // Write results to file if outputFile is specified
    if (options.outputFile) {
      try {
        await fs.writeFile(
          options.outputFile,
          JSON.stringify(result, null, 2),
          "utf-8"
        );
        console.log(`\n📝 Results written to: ${options.outputFile}`);
      } catch (error) {
        console.error(
          `⚠️  Failed to write results to file: ${
            error instanceof Error ? error.message : String(error)
          }`
        );
      }
    }

    return result;
  } catch (error) {
    const duration = Date.now() - startTime;
    const result: CursorAgentResult = {
      success: false,
      output: "",
      error: error instanceof Error ? error.message : String(error),
      duration,
      buildSuccess: false,
      lintSuccess: false,
      testSuccess: false,
      evalPath,
      timestamp: new Date().toISOString()
    };

    // Write error results to file if outputFile is specified
    if (options.outputFile) {
      try {
        await fs.writeFile(
          options.outputFile,
          JSON.stringify(result, null, 2),
          "utf-8"
        );
        console.log(`\n📝 Results written to: ${options.outputFile}`);
      } catch (writeError) {
        console.error(
          `⚠️  Failed to write results to file: ${
            writeError instanceof Error
              ? writeError.message
              : String(writeError)
          }`
        );
      }
    }

    return result;
  }
}

function buildCursorCommand(
  workspaceDir: string,
  prompt: string,
  options: CursorAgentOptions
): string {
  const args = ["cursor-agent"];

  // Add API key as flag (more reliable than env var)
  if (options.apiKey) {
    args.push("--api-key", options.apiKey);
  }

  // Add print mode for non-interactive execution
  args.push("-p");

  // Add model flag
  args.push("--model", "sonnet-4.5");

  // Add force flag if specified
  if (options.force) {
    args.push("--force");
  }

  // Add output format
  if (options.outputFormat) {
    args.push("--output-format", options.outputFormat);
  }

  // Append instruction to not run npm/pnpm commands to the prompt
  const enhancedPrompt = `${prompt}

IMPORTANT: Do not run npm, pnpm, yarn, or any package manager commands. Dependencies have already been installed. Do not run build, test, or dev server commands. Just write the code files. DO Not ask any followup questions either.`;

  // Add the prompt (escaped for shell)
  const escapedPrompt = enhancedPrompt.replace(/'/g, "'\\''");
  args.push(`'${escapedPrompt}'`);

  return args.join(" ");
}

function parseStreamJson(output: string): any[] {
  const lines = output.split("\n").filter((line) => line.trim());
  const streamData: any[] = [];

  for (const line of lines) {
    try {
      const data = JSON.parse(line);
      streamData.push(data);
    } catch {
      // Skip non-JSON lines
    }
  }

  return streamData;
}

async function runBuildCommand(
  workspaceDir: string,
  verbose: boolean
): Promise<{ success: boolean; output: string }> {
  try {
    // Check for package.json to determine project type
    const packageJsonPath = path.join(workspaceDir, "package.json");
    const hasPackageJson = await fs
      .stat(packageJsonPath)
      .then(() => true)
      .catch(() => false);

    if (!hasPackageJson) {
      if (verbose) console.log("    No package.json found, skipping build");
      return { success: true, output: "No package.json found, skipping build" };
    }

    // Read package.json to check for build script
    const packageJson = JSON.parse(await fs.readFile(packageJsonPath, "utf-8"));
    const hasBuildScript = packageJson.scripts?.build;

    if (!hasBuildScript) {
      if (verbose) console.log("    No build script found in package.json");
      return { success: true, output: "No build script found in package.json" };
    }

    if (verbose) {
      console.log("    🔨 Running build command...");
    }

    // Use workspace's node_modules binary
    const nextBin = path.join(workspaceDir, "node_modules", ".bin", "next");

    const { stdout, stderr, exitCode } = await execAsync(`"${nextBin}" build`, {
      cwd: workspaceDir,
      timeout: 120000, // 2 minute timeout for build
      verbose,
      idleTimeout: 30000 // 30 second idle timeout
    });

    return {
      success: exitCode === 0,
      output: exitCode === 0 ? stdout : stderr || stdout
    };
  } catch (error) {
    return {
      success: false,
      output: error instanceof Error ? error.message : String(error)
    };
  }
}

async function runLintCommand(
  workspaceDir: string,
  verbose: boolean
): Promise<{ success: boolean; output: string }> {
  try {
    // Check for package.json
    const packageJsonPath = path.join(workspaceDir, "package.json");
    const hasPackageJson = await fs
      .stat(packageJsonPath)
      .then(() => true)
      .catch(() => false);

    if (!hasPackageJson) {
      if (verbose) console.log("    No package.json found, skipping lint");
      return { success: true, output: "No package.json found, skipping lint" };
    }

    // Read package.json to check for lint script
    const packageJson = JSON.parse(await fs.readFile(packageJsonPath, "utf-8"));
    const hasLintScript = packageJson.scripts?.lint;

    if (!hasLintScript) {
      if (verbose) console.log("    No lint script found in package.json");
      return { success: true, output: "No lint script found in package.json" };
    }

    if (verbose) {
      console.log("    🔍 Running lint command...");
    }

    // Use workspace's node_modules binary
    const nextBin = path.join(workspaceDir, "node_modules", ".bin", "next");

    const { stdout, stderr, exitCode } = await execAsync(`"${nextBin}" lint`, {
      cwd: workspaceDir,
      timeout: 60000, // 1 minute timeout for lint
      verbose,
      idleTimeout: 30000 // 30 second idle timeout
    });

    return {
      success: exitCode === 0,
      output: exitCode === 0 ? stdout : stderr || stdout
    };
  } catch (error) {
    return {
      success: false,
      output: error instanceof Error ? error.message : String(error)
    };
  }
}

async function runTestCommand(
  workspaceDir: string,
  verbose: boolean
): Promise<{ success: boolean; output: string }> {
  try {
    // Check for package.json
    const packageJsonPath = path.join(workspaceDir, "package.json");
    const hasPackageJson = await fs
      .stat(packageJsonPath)
      .then(() => true)
      .catch(() => false);

    if (!hasPackageJson) {
      if (verbose) console.log("    No package.json found, skipping tests");
      return { success: true, output: "No package.json found, skipping tests" };
    }

    // Read package.json to check for test script
    const packageJson = JSON.parse(await fs.readFile(packageJsonPath, "utf-8"));
    const hasTestScript = packageJson.scripts?.test;

    if (!hasTestScript) {
      if (verbose) console.log("    No test script found in package.json");
      return { success: true, output: "No test script found in package.json" };
    }

    if (verbose) {
      console.log("    🧪 Running test command...");
    }

    // Use workspace's node_modules binary
    const vitestBin = path.join(workspaceDir, "node_modules", ".bin", "vitest");

    const { stdout, stderr, exitCode } = await execAsync(`"${vitestBin}" run`, {
      cwd: workspaceDir,
      timeout: 180000, // 3 minute timeout for tests
      verbose,
      idleTimeout: 30000 // 30 second idle timeout
    });

    return {
      success: exitCode === 0,
      output: exitCode === 0 ? stdout : stderr || stdout
    };
  } catch (error) {
    return {
      success: false,
      output: error instanceof Error ? error.message : String(error)
    };
  }
}
