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
    timeout: 18_000_000,
  });
  console.log(`Sandbox created: ${sandbox.sandboxId}`);

  try {
    console.log("Installing Rust and cargo-audit...");
    await sandbox.runCommand({
      cmd: "dnf",
      args: ["install", "-y", "rust", "cargo", "gcc", "openssl-devel"],
      sudo: true,
    });
    await sandbox.runCommand("cargo", ["install", "cargo-audit"]);

    console.log("Installing pnpm...");
    await sandbox.runCommand("npm", ["install", "-g", "pnpm@10"]);

    console.log("Cloning repo...");
    await sandbox.runCommand("git", [
      "clone",
      "--depth",
      "1",
      REPO_URL,
      "turborepo",
    ]);

    console.log("Installing agent dependencies...");
    await sandbox.runCommand("npm", ["install", "ai", "zod"]);

    console.log("Uploading agent script...");
    const agentScript = readFileSync("sandbox/audit-fix-agent.mjs");
    await sandbox.writeFiles([
      { path: "/vercel/sandbox/audit-fix-agent.mjs", content: agentScript },
    ]);

    console.log("Running agent...\n---");
    const cmd = await sandbox.runCommand({
      cmd: "bash",
      args: [
        "-c",
        `AI_GATEWAY_API_KEY=${AI_GATEWAY_API_KEY} node audit-fix-agent.mjs`,
      ],
      detached: true,
    });

    for await (const log of cmd.logs()) {
      if (log.stream === "stdout") {
        process.stdout.write(log.data);
      } else {
        process.stderr.write(log.data);
      }
    }

    const result = await cmd.wait();
    console.log("---\nAgent exited with code:", result.exitCode);

    // Read results
    try {
      const resultsBuffer = await sandbox.readFileToBuffer({
        path: "/vercel/sandbox/results.json",
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
      "cd turborepo && git diff",
    ]);
    const diff = await diffResult.stdout();
    if (diff) {
      console.log("\n=== Diff ===");
      console.log(diff);
    } else {
      console.log("\nNo changes made.");
    }
  } finally {
    // Let streams drain before tearing down the sandbox connection
    await new Promise((resolve) => setTimeout(resolve, 1000));
    console.log("\nStopping sandbox...");
    await sandbox.stop();
    console.log("Done.");
  }
}

main().catch((err) => {
  console.error("Fatal:", err);
  process.exit(1);
});
