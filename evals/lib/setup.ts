import { readFileSync } from "node:fs";
import type { Sandbox } from "@vercel/agent-eval";

/** Packaged docs come from this checkout. The native binary comes from the
 * matching published @turbo/<platform> optional dependency in the sandbox. */
export async function installTurbo(sandbox: Sandbox): Promise<void> {
  const tarball = process.env.TURBO_EVAL_TARBALL;
  if (!tarball)
    throw new Error(
      "No turbo tarball. Run via `pnpm eval` at the repository root."
    );

  await sandbox.writeFiles({
    // @ts-expect-error The sandbox runtime accepts Buffers for binary files.
    "turbo.tgz": readFileSync(tarball)
  });
  const install = await sandbox.runCommand("npm", ["install", "./turbo.tgz"]);
  if (install.exitCode !== 0) {
    throw new Error(
      `Installing local turbo failed (${install.exitCode}): ${install.stderr}`
    );
  }
  const check = await sandbox.runCommand("node", [
    "-e",
    "const fs = require('fs'); if (!fs.existsSync('node_modules/turbo/docs/README.md')) process.exit(1)"
  ]);
  if (check.exitCode !== 0)
    throw new Error("Packed turbo is missing docs/README.md");
  const binary = await sandbox.runCommand("node_modules/.bin/turbo", [
    "--version"
  ]);
  if (binary.exitCode !== 0) {
    throw new Error(
      `Installed turbo binary cannot run (${binary.exitCode}): ${binary.stderr}`
    );
  }
  const manifest = JSON.parse(
    await sandbox.readFile("node_modules/turbo/package.json")
  );
  if (binary.stdout.trim() !== manifest.version) {
    throw new Error(
      `turbo package ${manifest.version} selected binary ${binary.stdout.trim()}`
    );
  }
}

export async function writeAgentsMd(sandbox: Sandbox): Promise<void> {
  await sandbox.writeFiles({
    "AGENTS.md": `<!-- BEGIN:turborepo-agent-rules -->
# Turborepo documentation

Before changing Turborepo tasks, caching, or workspace configuration, read
\`node_modules/turbo/docs/README.md\` and the smallest relevant pages it links to.
These docs match the installed \`turbo\` package; do not rely on remembered behavior.
<!-- END:turborepo-agent-rules -->
`,
    "CLAUDE.md": "@AGENTS.md\n",
    "GEMINI.md": "@AGENTS.md\n"
  });
}
