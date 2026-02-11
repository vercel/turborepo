// Run with: node --env-file .env.local sandbox/test-local.mjs
//
// Requires .env.local with:
//   AI_GATEWAY_API_KEY=...
//   GITHUB_TOKEN=...      (only needed if you want to push/PR, otherwise optional)

import { readFileSync } from "node:fs";
import { Sandbox } from "@vercel/sandbox";

const AI_GATEWAY_API_KEY = process.env.AI_GATEWAY_API_KEY;
if (!AI_GATEWAY_API_KEY) {
  console.error("Missing AI_GATEWAY_API_KEY in .env.local");
  process.exit(1);
}

const REPO_URL = "https://github.com/vercel/turborepo.git";

async function main() {
  console.log("Creating sandbox...");
  const sandbox = await Sandbox.create({
    runtime: "node22",
    resources: { vcpus: 4 },
    timeout: 18_000_000
  });
  console.log(`Sandbox created: ${sandbox.sandboxId}`);

  try {
    const CARGO_AUDIT_VERSION = "0.22.1";
    const CARGO_AUDIT_DIR = `cargo-audit-x86_64-unknown-linux-gnu-v${CARGO_AUDIT_VERSION}`;
    console.log("Installing cargo-audit (pre-built binary)...");
    await sandbox.runCommand("bash", [
      "-c",
      `curl -sL "https://github.com/rustsec/rustsec/releases/download/cargo-audit%2Fv${CARGO_AUDIT_VERSION}/${CARGO_AUDIT_DIR}.tgz" | tar xz -C /tmp && mv /tmp/${CARGO_AUDIT_DIR}/cargo-audit /usr/local/bin/cargo-audit && chmod +x /usr/local/bin/cargo-audit`,
    ]);

    console.log("Installing pnpm...");
    await sandbox.runCommand("npm", ["install", "-g", "pnpm@10"]);

    console.log("Cloning repo...");
    await sandbox.runCommand("git", [
      "clone",
      "--depth",
      "1",
      REPO_URL,
      "turborepo"
    ]);

    console.log("Installing agent dependencies...");
    await sandbox.runCommand("npm", ["install", "ai", "zod"]);

    console.log("Uploading agent script...");
    const agentScript = readFileSync("sandbox/audit-fix-agent.mjs");
    await sandbox.writeFiles([
      { path: "/vercel/sandbox/audit-fix-agent.mjs", content: agentScript }
    ]);

    console.log("Running agent...\n---");
    const result = await sandbox.runCommand({
      cmd: "bash",
      args: [
        "-c",
        `AI_GATEWAY_API_KEY=${AI_GATEWAY_API_KEY} node audit-fix-agent.mjs`
      ],
      stdout: process.stdout,
      stderr: process.stderr
    });

    console.log("---\nAgent exited with code:", result.exitCode);

    // Read results
    try {
      const resultsBuffer = await sandbox.readFileToBuffer({
        path: "/vercel/sandbox/results.json"
      });
      if (resultsBuffer) {
        const results = JSON.parse(resultsBuffer.toString("utf-8"));
        console.log("\n=== Agent Results ===");
        console.log(JSON.stringify(results, null, 2));
      }
    } catch {
      console.log("\nNo results.json produced.");
    }

    // Show the diff
    const diffResult = await sandbox.runCommand("bash", [
      "-c",
      "cd turborepo && git diff"
    ]);
    const diff = await diffResult.stdout();
    if (diff) {
      console.log("\n=== Diff ===");
      console.log(diff);
    } else {
      console.log("\nNo changes made.");
    }
  } finally {
    console.log("\nStopping sandbox...");
    try {
      await sandbox.stop();
    } catch {
      // Sandbox may already be stopped or connection closed
    }
    console.log("Done.");
  }
}

main().catch((err) => {
  console.error("Fatal:", err);
  process.exit(1);
});
