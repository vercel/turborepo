// This script runs INSIDE a Vercel Sandbox VM.
// It is uploaded and executed by the serverless function in lib/audit.ts.
//
// The sandbox has: node22, cargo-audit, pnpm, and the repo cloned at ./turborepo.
// AI_GATEWAY_API_KEY is passed as an env var.

import { ToolLoopAgent, tool, zodSchema, stepCountIs } from "ai";
import { execSync } from "node:child_process";
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { z } from "zod";

const REPO_DIR = process.env.REPO_DIR ?? "/vercel/sandbox/turborepo";
const RESULTS_PATH = process.env.RESULTS_PATH ?? "/vercel/sandbox/results.json";
const MAX_STEPS = 80;

function shell(cmd, { cwd = REPO_DIR, allowFailure = false } = {}) {
  try {
    return execSync(cmd, {
      cwd,
      encoding: "utf-8",
      timeout: 120_000,
      env: process.env
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
  toolChoice: "required",
  instructions: `You are a senior engineer fixing security vulnerabilities in the Turborepo monorepo.

The repo is cloned at ${REPO_DIR}. Tools available: cargo-audit (at /usr/local/bin/cargo-audit), pnpm, node.
Rust toolchain is NOT installed — do not try to install it or run cargo build/check/test.

RULES:
- ALWAYS use tools. Plain text terminates the loop. Use "think" to reason.
- Be action-oriented. Do not over-research. Make changes, then verify.
- Call "reportResults" when done. This is mandatory.
- You have ${MAX_STEPS} steps total. Budget them: ~5 for audit, ~5 for planning, ~20 for fixing, ~10 for testing, ~5 for re-audit.

STRATEGY — follow this order:
1. Run "pnpm audit --json" and "cargo-audit audit --json" to get the vulnerability list.
2. For each vulnerability, determine the fix:
   a. If a direct dependency can be bumped to a non-vulnerable version, update it in the relevant package.json or Cargo.toml.
   b. For transitive dependencies that can't be fixed by bumping the direct dep, add a pnpm override in the root package.json (under "pnpm.overrides") to force the patched version.
   c. For Cargo.toml, update the version constraint to require the patched version.
3. After editing manifests, run "pnpm install --no-frozen-lockfile" to regenerate the lockfile.
4. Run "pnpm audit" again to verify fixes.
5. Run tests for affected packages: "pnpm run check-types --filter=<package>" if available.
6. Call reportResults with a summary.

IMPORTANT:
- pnpm overrides go in the ROOT package.json under "pnpm": { "overrides": { "package": ">=version" } }.
- False positives: if a workspace package name matches an npm package name (e.g. a workspace called "cli" matching the npm "cli" package), skip it — that's a pnpm audit bug.
- Don't waste steps investigating whether an override will break something. Make the change, run tests, fix if broken.`,

  tools: {
    think: tool({
      description: "Reason or plan. Use instead of generating text.",
      inputSchema: zodSchema(
        z.object({
          thought: z.string().describe("Your reasoning")
        })
      ),
      execute: async ({ thought }) => {
        console.log(`[think] ${thought}`);
        return "Continue.";
      }
    }),

    runCommand: tool({
      description:
        "Run a shell command in the repo. Use allowFailure:true for commands that might fail (audits, tests).",
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
            .describe("Return output even on non-zero exit (default false)")
        })
      ),
      execute: async ({ command, cwd, allowFailure }) => {
        console.log(`$ ${command}`);
        const output = shell(command, {
          cwd: cwd ?? REPO_DIR,
          allowFailure: allowFailure ?? false
        });
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
      description: "Read a file in the repo.",
      inputSchema: zodSchema(
        z.object({
          path: z.string().describe("File path relative to repo root")
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
      description: "Find files matching a pattern.",
      inputSchema: zodSchema(
        z.object({
          pattern: z
            .string()
            .describe('Glob pattern, e.g. "packages/*/package.json"')
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
      description: "Write final results. MUST be called as the last action.",
      inputSchema: zodSchema(
        z.object({
          success: z.boolean().describe("Were all vulnerabilities resolved?"),
          summary: z
            .string()
            .describe("What was done and what the reviewer should know"),
          vulnerabilitiesFixed: z.number(),
          vulnerabilitiesRemaining: z.number(),
          manifestsUpdated: z
            .array(z.string())
            .describe("Manifest files modified (Cargo.toml, package.json)"),
          sourceFilesUpdated: z
            .array(z.string())
            .describe("Source files modified for compatibility"),
          testsPass: z.boolean(),
          auditsClean: z.boolean()
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
      prompt: `Run security audits on this repo and fix the vulnerabilities. Follow the strategy in your instructions exactly — audit, fix manifests, reinstall, verify, report. Do not over-analyze. Act quickly.`
    });

    console.log("\nAgent finished. Final text:", result.text);

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
