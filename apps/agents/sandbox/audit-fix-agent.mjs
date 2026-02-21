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
const MAX_STEPS = 200;

function shell(cmd, opts = {}) {
  const cwd = opts.cwd ?? REPO_DIR;
  const allowFailure = opts.allowFailure ?? false;
  try {
    return execSync(cmd, {
      cwd,
      encoding: "utf-8",
      timeout: 120_000,
      env: process.env
    }).trim();
  } catch (e) {
    if (allowFailure) {
      return [
        "EXIT CODE " + e.status,
        "STDOUT:",
        (e.stdout ?? "").trim(),
        "STDERR:",
        (e.stderr ?? "").trim()
      ].join("\n");
    }
    throw e;
  }
}

function truncate(text) {
  if (text.length > 15000) {
    return (
      text.slice(0, 7000) + "\n\n... [truncated] ...\n\n" + text.slice(-7000)
    );
  }
  return text;
}

const agent = new ToolLoopAgent({
  model: "anthropic/claude-opus-4-6",
  stopWhen: stepCountIs(MAX_STEPS),
  toolChoice: "required",
  instructions: [
    "You are a senior engineer fixing security vulnerabilities in the Turborepo monorepo.",
    "",
    "The repo is cloned at " +
      REPO_DIR +
      ". Tools available: cargo-audit, cargo, rustc, pnpm, node.",
    "Rust is installed via dnf. cargo-audit is at ~/.cargo/bin/cargo-audit. You can use cargo check/build if needed.",
    "",
    "RULES:",
    '- ALWAYS use tools. Plain text terminates the loop. Use "think" to reason.',
    "- Be action-oriented. Do not over-research. Make changes, then verify.",
    '- Call "reportResults" when done. This is mandatory — it stops the loop.',
    "- You may not update package manager lockfiles directly. Update manifests to clear the vulnerabilities.",
    "- You may update our source code to upgrade through majors or other changes as needed.",
    '- Avoid using hacks like "overrides" at all costs - only when we have no other option. You might even consider replacing the dependency entirely before using hacks.',
    "",
    "STRATEGY — follow this order:",
    '1. Run "pnpm audit --json" and "cargo-audit audit --json" to get the vulnerability list.',
    "2. For each vulnerability, determine the fix:",
    "   a. If a direct dependency can be bumped to a non-vulnerable version, update it in the relevant package.json or Cargo.toml.",
    "   b. For Cargo.toml, update the version constraint to require the patched version.",
    '3. After editing manifests, run "pnpm install --no-frozen-lockfile" to regenerate the lockfile.',
    '4. Run "pnpm audit" again to verify fixes.',
    '5. Run "cargo build" and "cargo test" to ensure the Rust code is working.',
    '5. Run tests for affected packages: "pnpm run check-types --filter=<package>" if available.',
    "6. Call reportResults with a summary.",
    "",
    "IMPORTANT:",
    '- pnpm overrides go in the ROOT package.json under "pnpm": { "overrides": { "package": ">=version" } }.',
    '- False positives: if a workspace package name matches an npm package name (e.g. a workspace called "cli" matching the npm "cli" package), skip it — that is a pnpm audit bug.',
    "- Don't waste steps investigating whether an override will break something. Make the change, run tests, fix if broken."
  ].join("\n"),

  tools: {
    think: tool({
      description: "Reason or plan. Use instead of generating text.",
      inputSchema: zodSchema(
        z.object({
          thought: z.string().describe("Your reasoning")
        })
      ),
      execute: async function ({ thought }) {
        console.log("[think] " + thought);
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
      execute: async function ({ command, cwd, allowFailure }) {
        console.log("$ " + command);
        const output = shell(command, {
          cwd: cwd ?? REPO_DIR,
          allowFailure: allowFailure ?? false
        });
        return truncate(output);
      }
    }),

    readFile: tool({
      description: "Read a file in the repo.",
      inputSchema: zodSchema(
        z.object({
          path: z.string().describe("File path relative to repo root")
        })
      ),
      execute: async function ({ path }) {
        const fullPath = REPO_DIR + "/" + path;
        if (!existsSync(fullPath)) {
          return "File not found: " + path;
        }
        return truncate(readFileSync(fullPath, "utf-8"));
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
      execute: async function ({ path, content }) {
        const fullPath = REPO_DIR + "/" + path;
        writeFileSync(fullPath, content, "utf-8");
        return "Wrote " + content.length + " bytes to " + path;
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
      execute: async function ({ pattern }) {
        const output = shell("find . -path './" + pattern + "' | head -50", {
          allowFailure: true
        });
        return output || "(no matches)";
      }
    }),

    reportResults: tool({
      description:
        "Write final results. MUST be called as the last action. This stops the agent loop.",
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
      )
      // No execute — calling this tool stops the agent loop.
    })
  }
});

async function main() {
  console.log("Starting audit fix agent...");

  try {
    const result = await agent.generate({
      prompt:
        "Run security audits on this repo and fix the vulnerabilities. Follow the strategy in your instructions exactly — audit, fix manifests, reinstall, verify, report. Do not over-analyze. Act quickly."
    });

    console.log("\nAgent finished.");

    const reportCall = result.steps
      .flatMap(function (s) {
        return s.toolCalls ?? [];
      })
      .find(function (tc) {
        return tc.toolName === "reportResults";
      });

    if (reportCall) {
      console.log("Results from reportResults tool call.");
      writeFileSync(
        RESULTS_PATH,
        JSON.stringify(reportCall.input, null, 2),
        "utf-8"
      );
    } else if (!existsSync(RESULTS_PATH)) {
      console.log("Agent did not call reportResults.");
      writeFileSync(
        RESULTS_PATH,
        JSON.stringify(
          {
            success: false,
            summary:
              "Agent completed without calling reportResults. Final text: " +
              result.text,
            vulnerabilitiesFixed: 0,
            vulnerabilitiesRemaining: -1,
            manifestsUpdated: [],
            sourceFilesUpdated: [],
            testsPass: false,
            auditsClean: false
          },
          null,
          2
        ),
        "utf-8"
      );
    }
  } catch (err) {
    console.error("Agent error:", err);
    writeFileSync(
      RESULTS_PATH,
      JSON.stringify(
        {
          success: false,
          summary: "Agent crashed: " + (err.message ?? String(err)),
          vulnerabilitiesFixed: 0,
          vulnerabilitiesRemaining: -1,
          manifestsUpdated: [],
          sourceFilesUpdated: [],
          testsPass: false,
          auditsClean: false
        },
        null,
        2
      ),
      "utf-8"
    );
  }
}

main();
