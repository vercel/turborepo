// This script runs INSIDE a Vercel Sandbox VM.
// It is uploaded and executed by the serverless function in lib/audit.ts.
//
// The sandbox has: node22, pnpm, rust + cargo-audit, and the repo cloned at ./turborepo.
// AI_GATEWAY_API_KEY is passed as an env var.

import { ToolLoopAgent, tool, zodSchema, stepCountIs } from "ai";
import { execSync } from "node:child_process";
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { z } from "zod";

const REPO_DIR = "/vercel/sandbox/turborepo";
const RESULTS_PATH = "/vercel/sandbox/results.json";
const MAX_STEPS = 30;

function shell(cmd, { cwd = REPO_DIR, allowFailure = false } = {}) {
  try {
    return execSync(cmd, {
      cwd,
      encoding: "utf-8",
      timeout: 120_000,
      env: {
        ...process.env,
        PATH: `${process.env.HOME}/.cargo/bin:${process.env.PATH}`
      }
    }).trim();
  } catch (e) {
    if (allowFailure) {
      return `EXIT CODE ${e.status}\nSTDOUT:\n${e.stdout?.trim() ?? ""}\nSTDERR:\n${e.stderr?.trim() ?? ""}`;
    }
    throw e;
  }
}

const agent = new ToolLoopAgent({
  model: "anthropic/claude-opus-4-6",
  stopWhen: stepCountIs(MAX_STEPS),
  instructions: `You are a senior engineer fixing security vulnerabilities in the Turborepo monorepo.

The repo is already cloned at ${REPO_DIR}. Rust, cargo-audit, pnpm, and node are installed.

Your job:
1. Run security audits (cargo audit, pnpm audit) to identify vulnerabilities.
2. Fix them by updating dependency version constraints in manifest files (Cargo.toml, package.json).
   Do NOT just update lockfiles — that is not a fix.
3. After making changes, run the relevant test suites to make sure nothing is broken.
   - For Rust: cargo check (full build takes too long, check is sufficient)
   - For JS/TS: pnpm run check-types (if it exists), pnpm test --filter=<affected>
4. If tests fail, read the errors, diagnose the issue, and fix the source code as needed.
5. Re-run audits to verify the vulnerabilities are resolved.
6. Repeat until clean or you've exhausted your options.

When you are done, use the reportResults tool to write the final summary. This is mandatory.

Important:
- The repo is a monorepo with Rust crates in crates/ and JS/TS packages in packages/ and apps/.
- Cargo.toml workspace is at the root. Individual crates have their own Cargo.toml.
- pnpm-workspace.yaml defines the JS workspace.
- Be conservative. Don't bump major versions unless the audit specifically requires it.
- If a vulnerability cannot be auto-fixed, note it in your report rather than making risky changes.
- Always explain what you changed and why.`,

  tools: {
    runCommand: tool({
      description:
        "Run a shell command in the repo directory. Use for audits, builds, tests, git operations, etc.",
      inputSchema: zodSchema(
        z.object({
          command: z.string().describe("The shell command to run"),
          cwd: z
            .string()
            .optional()
            .describe("Working directory (defaults to repo root)"),
          allowFailure: z
            .boolean()
            .optional()
            .describe(
              "If true, returns output even on non-zero exit (default false)"
            )
        })
      ),
      execute: async ({ command, cwd, allowFailure }) => {
        console.log(`$ ${command}`);
        const output = shell(command, {
          cwd: cwd ?? REPO_DIR,
          allowFailure: allowFailure ?? false
        });
        // Truncate very long output to avoid blowing up context
        if (output.length > 15000) {
          return (
            output.slice(0, 7000) +
            "\n\n... [truncated] ...\n\n" +
            output.slice(-7000)
          );
        }
        return output;
      }
    }),

    readFile: tool({
      description: "Read the contents of a file in the repo.",
      inputSchema: zodSchema(
        z.object({
          path: z
            .string()
            .describe(
              "File path relative to repo root, e.g. crates/turborepo-lib/Cargo.toml"
            )
        })
      ),
      execute: async ({ path }) => {
        const fullPath = `${REPO_DIR}/${path}`;
        if (!existsSync(fullPath)) {
          return `File not found: ${path}`;
        }
        const content = readFileSync(fullPath, "utf-8");
        if (content.length > 15000) {
          return (
            content.slice(0, 7000) +
            "\n\n... [truncated] ...\n\n" +
            content.slice(-7000)
          );
        }
        return content;
      }
    }),

    writeFile: tool({
      description: "Write content to a file in the repo.",
      inputSchema: zodSchema(
        z.object({
          path: z.string().describe("File path relative to repo root"),
          content: z.string().describe("The full file content to write")
        })
      ),
      execute: async ({ path, content }) => {
        const fullPath = `${REPO_DIR}/${path}`;
        writeFileSync(fullPath, content, "utf-8");
        return `Wrote ${content.length} bytes to ${path}`;
      }
    }),

    listFiles: tool({
      description:
        "List files matching a glob pattern. Useful for finding Cargo.toml or package.json files.",
      inputSchema: zodSchema(
        z.object({
          pattern: z
            .string()
            .describe(
              'Glob pattern, e.g. "crates/*/Cargo.toml" or "packages/*/package.json"'
            )
        })
      ),
      execute: async ({ pattern }) => {
        const output = shell(`find . -path './${pattern}' | head -50`, {
          allowFailure: true
        });
        return output || "(no matches)";
      }
    }),

    reportResults: tool({
      description:
        "Write the final results JSON. Call this when you are done. This is mandatory.",
      inputSchema: zodSchema(
        z.object({
          success: z
            .boolean()
            .describe("Whether all vulnerabilities were resolved"),
          summary: z
            .string()
            .describe(
              "Human-readable summary of what was done and what the reviewer should know"
            ),
          vulnerabilitiesFixed: z
            .number()
            .describe("Number of vulnerabilities fixed"),
          vulnerabilitiesRemaining: z
            .number()
            .describe("Number that could not be auto-fixed"),
          manifestsUpdated: z
            .array(z.string())
            .describe(
              "List of manifest files that were modified (Cargo.toml, package.json)"
            ),
          sourceFilesUpdated: z
            .array(z.string())
            .describe(
              "List of source files that were modified for compatibility"
            ),
          testsPass: z.boolean().describe("Whether tests passed after changes"),
          auditsClean: z
            .boolean()
            .describe("Whether re-running audits shows 0 vulnerabilities")
        })
      ),
      execute: async (results) => {
        writeFileSync(RESULTS_PATH, JSON.stringify(results, null, 2), "utf-8");
        return "Results written. Agent complete.";
      }
    })
  }
});

async function main() {
  console.log("Starting audit fix agent...");

  try {
    const result = await agent.generate({
      prompt:
        "Run security audits on this repo, fix the vulnerabilities, verify the fixes with tests, and report the results."
    });

    console.log("\nAgent finished. Final text:", result.text);

    // Ensure results were written even if agent forgot to call reportResults
    if (!existsSync(RESULTS_PATH)) {
      writeFileSync(
        RESULTS_PATH,
        JSON.stringify({
          success: false,
          summary: `Agent completed without calling reportResults. Final text: ${result.text}`,
          vulnerabilitiesFixed: 0,
          vulnerabilitiesRemaining: -1,
          manifestsUpdated: [],
          sourceFilesUpdated: [],
          testsPass: false,
          auditsClean: false
        }),
        "utf-8"
      );
    }
  } catch (err) {
    console.error("Agent error:", err);
    writeFileSync(
      RESULTS_PATH,
      JSON.stringify({
        success: false,
        summary: `Agent crashed: ${err.message ?? String(err)}`,
        vulnerabilitiesFixed: 0,
        vulnerabilitiesRemaining: -1,
        manifestsUpdated: [],
        sourceFilesUpdated: [],
        testsPass: false,
        auditsClean: false
      }),
      "utf-8"
    );
  }
}

main();
