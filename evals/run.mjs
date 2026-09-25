#!/usr/bin/env node
import { execFileSync, spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  renameSync
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { ungradedAttempts } from "./lib/results-status.mjs";

const evalRoot = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(evalRoot, "..");
const fixtureRoot = join(evalRoot, "evals");
const variants = ["baseline", "agents-md"];
const usage = `Usage: pnpm eval <fixture> [--dry] [--variant baseline|agents-md] [--runs N]
       pnpm eval --all [--dry] [--variant baseline|agents-md] [--runs N]`;

function fail(message) {
  console.error(`${message}\n${usage}`);
  process.exit(1);
}

function fixtureNames() {
  return readdirSync(fixtureRoot, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name)
    .sort();
}

function parseArgs(args) {
  let fixture;
  let all = false;
  let dry = false;
  let selectedVariant;
  let runs = 1;
  for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    if (arg === "--all") all = true;
    else if (arg === "--dry") dry = true;
    else if (arg === "--variant") selectedVariant = args[++i];
    else if (arg === "--runs") runs = Number(args[++i]);
    else if (arg.startsWith("-")) fail(`Unknown option: ${arg}`);
    else if (!fixture) fixture = arg;
    else fail(`Unexpected argument: ${arg}`);
  }
  if (all === Boolean(fixture)) {
    fail(
      `Select one fixture or --all. Available: ${fixtureNames().join(", ")}`
    );
  }
  if (fixture && !fixtureNames().includes(fixture)) {
    fail(
      `Unknown fixture: ${fixture}. Available: ${fixtureNames().join(", ")}`
    );
  }
  if (selectedVariant && !variants.includes(selectedVariant)) {
    fail(`Unknown variant: ${selectedVariant}`);
  }
  if (!Number.isInteger(runs) || runs < 1)
    fail("--runs must be a positive integer");
  return {
    fixture,
    dry,
    runs,
    selectedVariants: selectedVariant ? [selectedVariant] : variants
  };
}

function packTurbo() {
  const directory = join(evalRoot, ".tarballs");
  mkdirSync(directory, { recursive: true });
  const output = execFileSync(
    "pnpm",
    ["pack", "--pack-destination", directory],
    {
      cwd: join(repoRoot, "packages/turbo"),
      encoding: "utf8"
    }
  );
  const produced = output.trim().split("\n").pop();
  if (!produced?.endsWith(".tgz"))
    throw new Error(`Unexpected pnpm pack output: ${output}`);
  const source = resolve(directory, produced);
  const destination = join(directory, "turbo.tgz");
  if (source !== destination) renameSync(source, destination);
  return destination;
}

function dockerHost() {
  if (process.env.DOCKER_HOST) return process.env.DOCKER_HOST;
  try {
    // dockerode does not honor the Docker CLI's active context on its own.
    return execFileSync(
      "docker",
      ["context", "inspect", "--format", "{{.Endpoints.docker.Host}}"],
      { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }
    ).trim();
  } catch {
    // Vercel Sandbox users may not have the Docker CLI installed.
    return undefined;
  }
}

function main() {
  const { fixture, dry, runs, selectedVariants } = parseArgs(
    process.argv.slice(2)
  );
  const bin = join(repoRoot, "node_modules/.bin/agent-eval");
  if (!existsSync(bin)) fail("agent-eval is not installed. Run pnpm install.");
  mkdirSync(join(evalRoot, "results"), { recursive: true });

  const version = JSON.parse(
    readFileSync(join(repoRoot, "packages/turbo/package.json"), "utf8")
  ).version;
  console.log(
    `turbo ${version}: ${fixture ?? "all fixtures"} (${selectedVariants.join(" + ")}, ${runs} run(s))`
  );
  if (dry) {
    console.log(
      "Dry run: no sandbox or model calls (local package not packed)."
    );
    const ungraded = ungradedAttempts(
      join(evalRoot, "results"),
      selectedVariants,
      fixture ? [fixture] : fixtureNames()
    );
    if (ungraded.length) {
      console.error(
        `Ungraded attempts (no eval output): ${ungraded.join(", ")}`
      );
      console.error(
        "These are not model failures. Fix the infrastructure error and rerun."
      );
      process.exitCode = 1;
      return;
    }
  }
  // For real runs, always force: the runner's fixture fingerprint does not include
  // the packed turbo package, and documentation changes must not reuse old results.
  const tarball = dry ? undefined : packTurbo();
  const endpoint = dry ? undefined : dockerHost();
  const result = spawnSync(
    bin,
    dry
      ? ["status", ...selectedVariants]
      : ["run", ...selectedVariants, "--force"],
    {
      cwd: evalRoot,
      stdio: "inherit",
      env: {
        ...process.env,
        EVAL_FILTER: fixture ?? "*",
        EVAL_RUNS: String(runs),
        ...(endpoint ? { DOCKER_HOST: endpoint } : {}),
        ...(tarball ? { TURBO_EVAL_TARBALL: tarball } : {})
      }
    }
  );
  if (result.error) throw result.error;
  process.exitCode = result.status ?? 1;
}

main();
