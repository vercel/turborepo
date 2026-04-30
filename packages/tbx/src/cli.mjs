#!/usr/bin/env node

import { spawn, spawnSync } from "node:child_process";
import {
  existsSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { Sandbox } from "@vercel/sandbox";

const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = resolve(packageRoot, "../..");
const rootPackageJson = JSON.parse(
  readFileSync(join(repoRoot, "package.json"), "utf8")
);
const toolPackageJson = JSON.parse(
  readFileSync(join(packageRoot, "package.json"), "utf8")
);

const defaults = {
  prefix: "turbo",
  baseSandbox: "turbo-base",
  repoPath: "/vercel/sandbox/src/turbo",
  defaultTimeout: "30m",
  snapshotExpiration: "14d",
  baseSnapshotExpiration: "none",
  runtime: "node22",
  vcpus: "32"
};

const userProfiles = {
  "anthony-shew": {
    dotfiles: {
      repo: "https://github.com/anthonyshew/dotfiles.git",
      install: "./bootstrap-linux.sh"
    }
  }
};

function usage() {
  return `Usage: pnpm tbx <command> [args]

Commands:
  setup                 Create repo-local config and verify sandbox is installed
  login                 Log in to the Sandbox CLI
  auth                  Show repo-local auth/project status
  ls                    List Turborepo sandboxes
  creds github <name>   Apply credential brokering to a PR sandbox
  creds check <name>    Verify brokered auth in a PR sandbox
  new <name>            Create a PR sandbox from the latest base snapshot
  sh <name>             Connect to a PR sandbox
  run <name> -- <cmd>   Run a command in a PR sandbox
  stop <name>           Stop a PR sandbox session
  rm <name>             Permanently remove a PR sandbox
  base refresh          Create or refresh the base for origin/main's SHA
                         Use --dotfiles to refresh only mapped user dotfiles in the base
  base id               Print the current base sandbox name

Repo auth:
  tbx uses the repo-pinned Sandbox CLI. Run 'pnpm tbx login' once.
  Project/account resolution is owned by the Sandbox CLI and normal Vercel project context.
`;
}

function readJson(path, fallback) {
  if (!existsSync(path)) {
    return fallback;
  }
  return JSON.parse(readFileSync(path, "utf8"));
}

function readConfig() {
  return defaults;
}

function loadEnvFile(path) {
  if (!existsSync(path)) {
    return;
  }

  for (const line of readFileSync(path, "utf8").split("\n")) {
    const match = line.match(/^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*)?\s*$/);
    if (!match || match[0].trim().startsWith("#")) {
      continue;
    }

    const [, key, rawValue = ""] = match;
    if (process.env[key] !== undefined) {
      continue;
    }

    let value = rawValue.trim();
    if (
      (value.startsWith('"') && value.endsWith('"')) ||
      (value.startsWith("'") && value.endsWith("'"))
    ) {
      value = value.slice(1, -1);
    }
    process.env[key] = value;
  }
}

function loadPackageEnv() {
  loadEnvFile(join(packageRoot, ".env.local"));
}

function sandboxEnv() {
  return process.env;
}

function sandboxArgs(args) {
  return args;
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: options.cwd ?? repoRoot,
    env: sandboxEnv(),
    stdio: options.capture ? "pipe" : "inherit",
    encoding: "utf8"
  });

  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0 && !options.allowFailure) {
    if (options.capture && result.stderr) {
      process.stderr.write(result.stderr);
    }
    process.exit(result.status ?? 1);
  }
  return result;
}

function runAsync(command, args, options = {}) {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd ?? repoRoot,
      env: sandboxEnv(),
      stdio: "inherit"
    });

    child.on("error", reject);
    child.on("close", (status) => {
      if (status !== 0 && !options.allowFailure) {
        process.exit(status ?? 1);
      }
      resolvePromise({ status });
    });
  });
}

function sandbox(args, options = {}) {
  return run(sandboxBinPath(), sandboxArgs(args), {
    ...options,
    cwd: packageRoot
  });
}

function sandboxAsync(args, options = {}) {
  return runAsync(sandboxBinPath(), sandboxArgs(args), {
    ...options,
    cwd: packageRoot
  });
}

function sandboxBinPath() {
  const packageBin = join(packageRoot, "node_modules", ".bin", "sandbox");
  if (existsSync(packageBin)) {
    return packageBin;
  }
  return join(repoRoot, "node_modules", ".bin", "sandbox");
}

function sandboxBinExists() {
  return existsSync(sandboxBinPath());
}

function requireSandboxInstalled() {
  if (sandboxBinExists()) {
    return;
  }

  const version = toolPackageJson.dependencies?.sandbox;
  console.error(
    `sandbox ${version ?? ""} is declared in @turbo/tbx but not installed.\n\nRun:\n  pnpm install --frozen-lockfile --ignore-scripts --filter @turbo/tbx\n`
  );
  process.exit(1);
}

function taskSandboxName(config, name) {
  if (!name) {
    throw new Error("Missing sandbox name");
  }
  if (!/^[a-zA-Z0-9][a-zA-Z0-9._-]*$/.test(name)) {
    throw new Error(
      "Sandbox names may only contain letters, numbers, dot, dash, and underscore"
    );
  }
  return `${config.prefix}-${name}`;
}

function taskBranchName(name) {
  return name.replaceAll("_", "-");
}

function shellQuote(value) {
  return `'${String(value).replaceAll("'", `'"'"'`)}'`;
}

function latestSnapshot(config) {
  const base = latestBase(config);
  if (base?.snapshotId) {
    warnIfBaseIsOutOfDate(config, base);
    return base.snapshotId;
  }
  console.error(`No base snapshot found. Run: pnpm tbx base refresh`);
  process.exit(1);
}

function snapshotExpirationValue(value) {
  return value === "none" ? "0" : value;
}

function snapshotIdFromOutput(output) {
  return output.match(/snap_[a-zA-Z0-9_]+/)?.[0] ?? null;
}

function latestSnapshotFromList() {
  const snapshots = sandbox(["snapshots", "list"], { capture: true });
  const ids = snapshots.stdout.match(/snap_[a-zA-Z0-9_]+/g) ?? [];
  if (ids.length === 0) {
    return null;
  }
  return ids[0];
}

function gitOutput(args) {
  const result = run("git", args, { capture: true });
  return result.stdout.trim();
}

function vercelUser() {
  let result;
  try {
    result = run("vercel", ["whoami", "--format", "json"], {
      capture: true,
      allowFailure: true
    });
  } catch {
    return null;
  }
  if (result.status !== 0 || !result.stdout.trim()) {
    return null;
  }

  try {
    return JSON.parse(result.stdout);
  } catch {
    return null;
  }
}

function userProfile() {
  const user = vercelUser();
  if (!user?.username) {
    return { user, profile: null };
  }
  return { user, profile: userProfiles[user.username] ?? null };
}

function repoRemote() {
  const remote = gitOutput(["config", "--get", "remote.origin.url"]);
  const githubSsh = remote.match(/^git@github\.com:(.+)$/);
  if (githubSsh) {
    return `https://github.com/${githubSsh[1]}`;
  }

  try {
    const url = new URL(remote);
    url.username = "";
    url.password = "";
    return url.toString();
  } catch {
    // Non-URL remotes are passed through for Git to validate.
  }

  return remote;
}

function durationMs(value) {
  const match = String(value).match(/^(\d+)(ms|s|m|h|d)$/);
  if (!match) {
    throw new Error(`Unsupported duration: ${value}`);
  }

  const amount = Number(match[1]);
  const unit = match[2];
  const multipliers = {
    ms: 1,
    s: 1000,
    m: 60 * 1000,
    h: 60 * 60 * 1000,
    d: 24 * 60 * 60 * 1000
  };

  return amount * multipliers[unit];
}

function mainSha(config) {
  const output = gitOutput(["ls-remote", repoRemote(), "refs/heads/main"]);
  const sha = output.split(/\s+/)[0];
  if (!/^[a-f0-9]{40}$/.test(sha)) {
    throw new Error(`Could not resolve main SHA for ${repoRemote()}`);
  }
  return sha;
}

function baseSandboxPrefix(config) {
  const { user, profile } = userProfile();
  const userSegment = profile ? `${user.username}-` : "";
  return `${config.baseSandbox}-${userSegment}`;
}

function baseSandboxName(config) {
  return `${baseSandboxPrefix(config)}${mainSha(config).slice(0, 12)}`;
}

function sandboxLine(name) {
  const result = sandbox(
    ["list", "--all", "--name-prefix", name, "--sort-by", "name"],
    { capture: true, allowFailure: true }
  );
  if (result.status !== 0) {
    return null;
  }
  return (
    result.stdout
      .split("\n")
      .find((line) => line.trim().startsWith(`${name} `)) ?? null
  );
}

function currentBase(config) {
  const name = baseSandboxName(config);
  const line = sandboxLine(name);
  return {
    name,
    line,
    snapshotId: line?.match(/snap_[a-zA-Z0-9_]+/)?.[0] ?? null
  };
}

function baseSandboxes(config) {
  const prefix = baseSandboxPrefix(config);
  const result = sandbox(
    ["list", "--all", "--sort-by", "statusUpdatedAt", "--sort-order", "desc"],
    { capture: true, allowFailure: true }
  );
  if (result.status !== 0) {
    return [];
  }

  return result.stdout
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.startsWith(prefix))
    .map((line) => {
      const [name] = line.split(/\s+/);
      return {
        name,
        line,
        sha: name.slice(prefix.length),
        snapshotId: line.match(/snap_[a-zA-Z0-9_]+/)?.[0] ?? null
      };
    })
    .filter((base) => /^[a-f0-9]{12}$/.test(base.sha));
}

function latestBase(config) {
  return baseSandboxes(config).find((base) => base.snapshotId) ?? null;
}

function warnIfBaseIsOutOfDate(config, base) {
  const currentSha = mainSha(config);
  if (base.sha === currentSha.slice(0, 12)) {
    return;
  }

  console.warn(
    `[tbx] warning: latest base snapshot is ${base.name}, but origin/main is ${currentSha.slice(0, 12)}. Run 'pnpm tbx base refresh' when you want a fresh base.`
  );
}

function sandboxExists(name) {
  const result = sandbox(
    ["list", "--all", "--name-prefix", name, "--sort-by", "name"],
    { capture: true, allowFailure: true }
  );

  if (result.status !== 0) {
    return false;
  }
  return result.stdout
    .split("\n")
    .some((line) => line.trim().startsWith(`${name} `));
}

function optionalCapture(command, args) {
  try {
    return run(command, args, { capture: true, allowFailure: true });
  } catch {
    return null;
  }
}

function hostGitHubToken() {
  const envToken = process.env.GH_TOKEN || process.env.GITHUB_TOKEN;
  if (envToken) {
    return envToken;
  }

  const result = optionalCapture("gh", ["auth", "token"]);
  const token = result?.stdout.trim();
  if (result?.status === 0 && token) {
    return token;
  }

  throw new Error(
    "GitHub credential brokering requires host GitHub auth. Run `gh auth login` locally, then retry."
  );
}

function hasHostGitHubToken() {
  try {
    return Boolean(hostGitHubToken());
  } catch {
    return false;
  }
}

function requireBrokeredCredentials() {
  const missing = [];

  try {
    hostGitHubToken();
  } catch {
    missing.push(
      "GitHub host auth: set GH_TOKEN/GITHUB_TOKEN or run `gh auth login` locally."
    );
  }

  if (!process.env.VERCEL_OIDC_TOKEN) {
    missing.push(
      "Vercel OIDC auth: add VERCEL_OIDC_TOKEN to packages/tbx/.env.local."
    );
  }

  if (missing.length > 0) {
    throw new Error(
      `Credential brokering requires host credentials before continuing:\n\n${missing
        .map((item) => `- ${item}`)
        .join("\n")}`
    );
  }
}

function githubCredentialPolicy() {
  const token = hostGitHubToken();
  const basic = Buffer.from(`x-access-token:${token}`).toString("base64");
  const bearerRule = [
    {
      transform: [{ headers: { authorization: `Bearer ${token}` } }]
    }
  ];
  const basicRule = [
    {
      transform: [{ headers: { authorization: `Basic ${basic}` } }]
    }
  ];

  const allow = {
    "api.github.com": bearerRule,
    "uploads.github.com": bearerRule,
    "github.com": basicRule,
    "codeload.github.com": basicRule,
    "gist.github.com": [],
    "objects.githubusercontent.com": [],
    "raw.githubusercontent.com": [],
    "release-assets.githubusercontent.com": [],
    "*.githubusercontent.com": []
  };

  return { allow };
}

function hostVercelOidcToken() {
  const token = process.env.VERCEL_OIDC_TOKEN;
  if (token) {
    return token;
  }

  throw new Error(
    "Vercel OIDC credential brokering requires VERCEL_OIDC_TOKEN in packages/tbx/.env.local."
  );
}

function maybeHostVercelOidcToken() {
  return process.env.VERCEL_OIDC_TOKEN || null;
}

function hasHostVercelOidcToken() {
  try {
    return Boolean(hostVercelOidcToken());
  } catch {
    return false;
  }
}

function brokeredVercelOidcToken() {
  const token = maybeHostVercelOidcToken();
  if (!token) {
    return null;
  }

  const parts = token.split(".");
  if (parts.length !== 3 || !parts[0] || !parts[1]) {
    return "tbx-brokered";
  }

  return `${parts[0]}.${parts[1]}.tbx-brokered`;
}

function vercelCredentialPolicy() {
  const token = maybeHostVercelOidcToken();
  if (!token) {
    return { allow: {} };
  }

  const oidcRule = [
    {
      transform: [
        {
          headers: {
            authorization: `Bearer ${token}`,
            "x-vercel-oidc-token": token
          }
        }
      ]
    }
  ];

  return {
    allow: {
      "ai-gateway.vercel.sh": oidcRule,
      "api.vercel.com": oidcRule,
      "oidc.vercel.com": oidcRule,
      "vercel.com": oidcRule,
      "*.vercel.com": oidcRule
    }
  };
}

function publicPackageRegistryPolicy() {
  return {
    allow: {
      "registry.npmjs.org": []
    }
  };
}

function credentialPolicy() {
  requireBrokeredCredentials();

  return {
    allow: {
      ...githubCredentialPolicy().allow,
      ...vercelCredentialPolicy().allow,
      ...publicPackageRegistryPolicy().allow
    }
  };
}

function brokeredGitHubEnvArgs() {
  const dummyToken = "tbx-brokered";
  const gitRewrite = `url.https://x-access-token:${dummyToken}@github.com/.insteadOf`;

  return [
    "--env",
    `GH_TOKEN=${dummyToken}`,
    "--env",
    `GITHUB_TOKEN=${dummyToken}`,
    "--env",
    "GIT_TERMINAL_PROMPT=0",
    "--env",
    "GIT_CONFIG_COUNT=2",
    "--env",
    `GIT_CONFIG_KEY_0=${gitRewrite}`,
    "--env",
    "GIT_CONFIG_VALUE_0=https://github.com/",
    "--env",
    `GIT_CONFIG_KEY_1=${gitRewrite}`,
    "--env",
    "GIT_CONFIG_VALUE_1=git@github.com:"
  ];
}

function brokeredCredentialEnvArgs() {
  requireBrokeredCredentials();

  const args = brokeredGitHubEnvArgs();
  const token = brokeredVercelOidcToken();
  if (token) {
    args.push("--env", `VERCEL_OIDC_TOKEN=${token}`);
    args.push("--env", "AI_GATEWAY_API_KEY=tbx-brokered");
  }
  return args;
}

function hostSigningProgram() {
  const result = optionalCapture("git", [
    "config",
    "--global",
    "--get",
    "gpg.ssh.program"
  ]);
  return result?.stdout.trim() || "ssh-keygen";
}

function hostSigningKey() {
  const result = optionalCapture("git", [
    "config",
    "--global",
    "--get",
    "user.signingkey"
  ]);
  const key = result?.stdout.trim();
  if (!key) {
    throw new Error(
      "Verified commit signing requires a global Git user.signingkey on the host."
    );
  }
  return key;
}

function hostSigningPublicKey() {
  const key = hostSigningKey();
  if (key.startsWith("key::")) {
    return key.slice("key::".length);
  }
  if (key.startsWith("ssh-")) {
    return key;
  }
  if (existsSync(key)) {
    if (key.endsWith(".pub")) {
      return readFileSync(key, "utf8").trim();
    }
    const result = run("ssh-keygen", ["-y", "-f", key], { capture: true });
    return result.stdout.trim();
  }
  throw new Error(`Could not resolve host SSH signing public key: ${key}`);
}

function signingShimCommand(config, publicKey) {
  const signingKey = `key::${publicKey}`;
  return `
set -euo pipefail
bin_dir="$HOME/.local/bin"
mkdir -p "$bin_dir"
cat > "$bin_dir/tbx-ssh-sign" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
sign_dir="/tmp/tbx-sign"
payload="\${@: -1}"
request_id="$(date +%s%N)-$$-$RANDOM"
request="$sign_dir/request.json"
response="$sign_dir/response-$request_id.sig"
mkdir -p "$sign_dir"
jq -n --arg id "$request_id" --rawfile payload "$payload" '{id: $id, payload: $payload}' > "$request.tmp.$request_id"
mv "$request.tmp.$request_id" "$request"
for _ in $(seq 1 600); do
  if [ -f "$response" ]; then
    base64 -d "$response" > "$payload.sig"
    rm -f "$response" "$request"
    exit 0
  fi
  sleep 0.2
done
echo "tbx signing broker timed out. Run commits through pnpm tbx sh/run." >&2
exit 1
EOF
chmod +x "$bin_dir/tbx-ssh-sign"
git config --global gpg.format ssh
git config --global user.signingkey ${shellQuote(signingKey)}
git config --global gpg.ssh.program "$bin_dir/tbx-ssh-sign"
git config --global commit.gpgsign true
git -C ${shellQuote(config.repoPath)} config gpg.format ssh
git -C ${shellQuote(config.repoPath)} config user.signingkey ${shellQuote(signingKey)}
git -C ${shellQuote(config.repoPath)} config gpg.ssh.program "$bin_dir/tbx-ssh-sign"
git -C ${shellQuote(config.repoPath)} config commit.gpgsign true
`;
}

function ensureSigningShim(config, sandboxName, publicKey) {
  console.log(
    `[tbx] configuring host-backed commit signing for ${sandboxName}`
  );
  sandbox([
    "exec",
    "--workdir",
    config.repoPath,
    ...brokeredCredentialEnvArgs(),
    sandboxName,
    "bash",
    "-lc",
    signingShimCommand(config, publicKey)
  ]);
}

function assertCommitSigningPayload(payload) {
  if (Buffer.byteLength(payload, "utf8") > 1024 * 1024) {
    throw new Error("Signing payload is too large");
  }
  if (
    !payload.startsWith("tree ") ||
    !payload.includes("\nauthor ") ||
    !payload.includes("\ncommitter ")
  ) {
    throw new Error("Signing broker only signs Git commit payloads");
  }
}

function signPayload(payload, publicKey, signer) {
  assertCommitSigningPayload(payload);
  const dir = mkdtempSync(join(tmpdir(), "tbx-sign-"));
  try {
    const payloadPath = join(dir, "payload");
    const keyPath = join(dir, "signing-key.pub");
    writeFileSync(payloadPath, payload);
    writeFileSync(keyPath, `${publicKey}\n`);
    run(signer, ["-Y", "sign", "-n", "git", "-f", keyPath, "-U", payloadPath], {
      capture: true
    });
    return readFileSync(`${payloadPath}.sig`).toString("base64");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

function sleep(ms) {
  return new Promise((resolvePromise) => setTimeout(resolvePromise, ms));
}

async function startSigningBroker(sandboxName) {
  const publicKey = hostSigningPublicKey();
  const signer = hostSigningProgram();
  const target = await Sandbox.get({ name: sandboxName });
  const seen = new Set();
  let closed = false;

  const loop = (async () => {
    while (true) {
      if (closed) {
        break;
      }

      const request = await target.readFileToBuffer({
        path: "/tmp/tbx-sign/request.json"
      });
      if (request) {
        const body = JSON.parse(request.toString("utf8"));
        if (
          typeof body.id === "string" &&
          typeof body.payload === "string" &&
          !seen.has(body.id)
        ) {
          seen.add(body.id);
          const signature = signPayload(body.payload, publicKey, signer);
          await target.writeFiles([
            {
              path: `/tmp/tbx-sign/response-${body.id}.sig`,
              content: signature
            }
          ]);
        }
      }
      await sleep(200);
    }
  })();

  loop.catch((error) => {
    if (!closed) {
      console.error(
        `[tbx] signing broker failed: ${error instanceof Error ? error.message : String(error)}`
      );
    }
  });

  return {
    publicKey,
    close() {
      closed = true;
    }
  };
}

async function applyGitHubCredentialBroker(config, name) {
  requireSandboxInstalled();
  const sandboxName = taskSandboxName(config, name);
  loadPackageEnv();
  requireBrokeredCredentials();
  console.log(`[tbx] applying credential brokering to ${sandboxName}`);
  const target = await Sandbox.get({ name: sandboxName });
  await target.updateNetworkPolicy(credentialPolicy());
}

async function checkGitHubCredentialBroker(name) {
  const config = readConfig();
  requireSandboxInstalled();
  const sandboxName = await ensureTaskSandbox(config, name);

  sandbox([
    "exec",
    "--workdir",
    config.repoPath,
    ...brokeredCredentialEnvArgs(),
    sandboxName,
    "bash",
    "-lc",
    "gh api user --jq .login && git ls-remote https://github.com/vercel/turbo.git HEAD >/dev/null && opencode providers list | grep -i vercel"
  ]);
}

function setup() {
  if (!sandboxBinExists()) {
    console.log(
      `Installing repo-pinned sandbox dependency (${toolPackageJson.dependencies.sandbox})...`
    );
    run("pnpm", [
      "install",
      "--frozen-lockfile",
      "--ignore-scripts",
      "--filter",
      "@turbo/tbx"
    ]);
  }
  authStatus();
}

function login() {
  requireSandboxInstalled();
  sandbox(["login"]);
}

function authStatus() {
  loadPackageEnv();
  const { user, profile } = userProfile();

  console.log(
    `sandbox: ${toolPackageJson.dependencies?.sandbox ?? "not declared"}`
  );
  console.log(`vercel user: ${user?.username ?? "not found"}`);
  console.log(`dotfiles: ${profile?.dotfiles ? "mapped" : "not mapped"}`);
  console.log(
    `github host auth: ${hasHostGitHubToken() ? "available" : "not found"}`
  );
  console.log(
    `vercel oidc token: ${hasHostVercelOidcToken() ? "available" : "not found"}`
  );
  console.log("project: resolved by Sandbox CLI");
  console.log("team: resolved by Sandbox CLI");
  console.log("sandbox cwd: packages/tbx");
  console.log("auth: Sandbox CLI login");
}

function listSandboxes() {
  const config = readConfig();
  requireSandboxInstalled();
  sandbox([
    "list",
    "--all",
    "--name-prefix",
    `${config.prefix}-`,
    "--sort-by",
    "name"
  ]);
}

async function createTask(name, publicKey = hostSigningPublicKey()) {
  const config = readConfig();
  requireSandboxInstalled();

  const sandboxName = taskSandboxName(config, name);
  const branch = taskBranchName(name);
  const snapshot = latestSnapshot(config);

  loadPackageEnv();
  console.log(
    `[tbx] creating ${sandboxName} from base snapshot with credential brokering`
  );
  await Sandbox.create({
    name: sandboxName,
    source: { type: "snapshot", snapshotId: snapshot },
    runtime: config.runtime,
    resources: { vcpus: Number(config.vcpus) },
    timeout: durationMs(config.defaultTimeout),
    snapshotExpiration: durationMs(config.snapshotExpiration),
    networkPolicy: credentialPolicy()
  });

  const command = `git switch -c ${shellQuote(branch)}`;

  sandbox([
    "exec",
    "--workdir",
    config.repoPath,
    sandboxName,
    "bash",
    "-lc",
    command
  ]);
  ensureSigningShim(config, sandboxName, publicKey);
}

async function ensureTaskSandbox(
  config,
  name,
  publicKey = hostSigningPublicKey()
) {
  const sandboxName = taskSandboxName(config, name);
  if (!sandboxExists(sandboxName)) {
    console.log(
      `[tbx] ${sandboxName} does not exist; creating from base snapshot`
    );
    await createTask(name, publicKey);
  } else {
    await applyGitHubCredentialBroker(config, name);
    ensureSigningShim(config, sandboxName, publicKey);
  }
  return sandboxName;
}

async function shell(name) {
  const config = readConfig();
  requireSandboxInstalled();
  const publicKey = hostSigningPublicKey();
  const sandboxName = await ensureTaskSandbox(config, name, publicKey);
  const broker = await startSigningBroker(sandboxName);
  try {
    await sandboxAsync([
      "exec",
      "--interactive",
      "--tty",
      "--workdir",
      config.repoPath,
      ...brokeredCredentialEnvArgs(),
      sandboxName,
      "bash",
      "-l"
    ]);
  } finally {
    broker.close();
  }
}

async function runInTask(name, command) {
  if (command[0] === "--") {
    command = command.slice(1);
  }
  if (command.length === 0) {
    console.error("Usage: pnpm tbx run <name> -- <cmd>");
    process.exit(1);
  }

  const config = readConfig();
  requireSandboxInstalled();
  const publicKey = hostSigningPublicKey();
  const sandboxName = await ensureTaskSandbox(config, name, publicKey);
  const broker = await startSigningBroker(sandboxName);
  try {
    await sandboxAsync([
      "exec",
      "--workdir",
      config.repoPath,
      ...brokeredCredentialEnvArgs(),
      sandboxName,
      ...command
    ]);
  } finally {
    broker.close();
  }
}

function stopTask(name) {
  const config = readConfig();
  requireSandboxInstalled();
  sandbox(["stop", taskSandboxName(config, name)]);
}

function removeTask(name) {
  const config = readConfig();
  requireSandboxInstalled();
  const sandboxName = taskSandboxName(config, name);
  sandbox(["remove", sandboxName]);
}

function dotfilesBootstrap(profile) {
  if (!profile?.dotfiles) {
    return "";
  }

  const repo = shellQuote(profile.dotfiles.repo);
  const install = shellQuote(profile.dotfiles.install);

  return `
step "install mapped dotfiles"
dotfiles_dir="$HOME/.dotfiles"
if [ -d "$dotfiles_dir/.git" ]; then
  step "git -C $dotfiles_dir pull --ff-only"
  git -C "$dotfiles_dir" pull --ff-only
else
  step "git clone ${profile.dotfiles.repo} $dotfiles_dir"
  git clone ${repo} "$dotfiles_dir"
fi
step "cd $dotfiles_dir"
cd "$dotfiles_dir"
step "${profile.dotfiles.install}"
bash -lc ${install}
`;
}

function snapshotBase(config, base) {
  console.log(`[tbx] Creating snapshot ${base.name}...`);
  const snapshot = sandbox(
    [
      "snapshot",
      base.name,
      "--stop",
      "--expiration",
      snapshotExpirationValue(config.baseSnapshotExpiration)
    ],
    { capture: true }
  );

  const id =
    snapshotIdFromOutput(`${snapshot.stdout}\n${snapshot.stderr}`) ??
    latestSnapshotFromList();
  if (!id) {
    process.stdout.write(`Created snapshot: ${snapshot.stdout}`);
    console.error("Could not find snapshot ID in sandbox output.");
    process.exit(1);
  }

  console.log(`${base.name} ${id}`);
}

function baseRefresh(args = []) {
  const installDotfiles = args.includes("--dotfiles");
  const unknown = args.filter((arg) => arg !== "--dotfiles");
  if (unknown.length > 0) {
    console.error(`Unknown base refresh option: ${unknown.join(" ")}`);
    process.exit(1);
  }

  const config = readConfig();
  requireSandboxInstalled();
  const { user, profile } = userProfile();
  const base = currentBase(config);

  if (installDotfiles && !profile?.dotfiles) {
    console.log(
      `[tbx] no dotfiles mapping for ${user?.username ?? "current Vercel user"}; skipping dotfiles`
    );
    return;
  }

  if (installDotfiles) {
    if (!base.line) {
      console.error(
        `No base sandbox found for ${base.name}. Run: pnpm tbx base refresh`
      );
      process.exit(1);
    }

    const command = `
set -euo pipefail
step() {
  printf '\n[tbx] %s\n' "$*"
}
${dotfilesBootstrap(profile)}
`;

    console.log(`[tbx] refreshing dotfiles in base sandbox ${base.name}`);
    sandbox(["run", "--name", base.name, "--", "bash", "-lc", command]);
    snapshotBase(config, base);
    return;
  }

  if (!base.line) {
    sandbox([
      "create",
      "--name",
      base.name,
      "--runtime",
      config.runtime,
      "--vcpus",
      config.vcpus,
      "--timeout",
      config.defaultTimeout,
      "--snapshot-expiration",
      config.baseSnapshotExpiration
    ]);
  }

  const command = `
set -euo pipefail
step() {
  printf '\n[tbx] %s\n' "$*"
}
if command -v apt-get >/dev/null 2>&1; then
  step "sudo apt-get update"
  sudo apt-get update
  step "sudo apt-get install -y build-essential curl git protobuf-compiler capnproto lld jq zstd"
  sudo apt-get install -y build-essential curl git protobuf-compiler capnproto lld jq zstd
elif command -v dnf >/dev/null 2>&1; then
  step "sudo dnf install -y gcc gcc-c++ make git protobuf-compiler lld jq zstd tar gzip"
  sudo dnf install -y gcc gcc-c++ make git protobuf-compiler lld jq zstd tar gzip
fi
if ! command -v curl >/dev/null 2>&1; then
  echo "Missing curl after dependency installation" >&2
  exit 1
fi
if ! command -v cc >/dev/null 2>&1; then
  echo "Missing C compiler after dependency installation" >&2
  exit 1
fi
if ! command -v capnp >/dev/null 2>&1; then
  capnp_version=1.0.2
  capnp_dir="/tmp/capnproto-c++-$capnp_version"
  step "curl -fsSL https://capnproto.org/capnproto-c++-$capnp_version.tar.gz | tar -xz -C /tmp"
  curl -fsSL "https://capnproto.org/capnproto-c++-$capnp_version.tar.gz" | tar -xz -C /tmp
  step "cd $capnp_dir"
  cd "$capnp_dir"
  step "./configure"
  ./configure
  step "make -j$(nproc)"
  make -j"$(nproc)"
  step "sudo make install"
  sudo make install
  step "sudo ldconfig"
  sudo ldconfig || true
fi
if ! command -v rustup >/dev/null 2>&1; then
  step "curl https://sh.rustup.rs -sSf | sh -s -- -y"
  curl https://sh.rustup.rs -sSf | sh -s -- -y
fi
step ". $HOME/.cargo/env"
. "$HOME/.cargo/env"
step "rustup update"
rustup update
step "corepack enable"
corepack enable
step "corepack prepare ${rootPackageJson.packageManager} --activate"
corepack prepare ${rootPackageJson.packageManager} --activate
step "npm install --global turbo@latest"
npm install --global turbo@latest
step "turbo --version"
turbo --version
step "mkdir -p ${shellQuote(dirname(config.repoPath))}"
mkdir -p ${shellQuote(dirname(config.repoPath))}
if [ ! -d ${shellQuote(join(config.repoPath, ".git"))} ]; then
  step "git clone ${repoRemote()} ${config.repoPath}"
  git clone ${shellQuote(repoRemote())} ${shellQuote(config.repoPath)}
fi
step "cd ${config.repoPath}"
cd ${shellQuote(config.repoPath)}
step "git checkout main"
git checkout main
step "git fetch origin"
git fetch origin
step "git reset --hard origin/main"
git reset --hard origin/main
step "pnpm install"
pnpm install
step "cargo fetch"
cargo fetch
step "cargo build"
cargo build
${dotfilesBootstrap(profile)}
`;

  console.log(`[tbx] refreshing base sandbox ${base.name}`);
  sandbox(["run", "--name", base.name, "--", "bash", "-lc", command]);
  snapshotBase(config, base);
}

function baseId() {
  const base = currentBase(readConfig());
  console.log(base.snapshotId ? `${base.name} ${base.snapshotId}` : base.name);
}

async function main() {
  const [command, ...args] = process.argv.slice(2);

  try {
    if (!command || command === "help" || command === "--help") {
      console.log(usage());
    } else if (command === "setup") {
      setup();
    } else if (command === "login") {
      login();
    } else if (command === "auth") {
      authStatus();
    } else if (command === "ls") {
      listSandboxes();
    } else if (command === "creds" && args[0] === "github") {
      await applyGitHubCredentialBroker(readConfig(), args[1]);
    } else if (command === "creds" && args[0] === "check") {
      await checkGitHubCredentialBroker(args[1]);
    } else if (command === "new") {
      await createTask(args[0]);
    } else if (command === "sh") {
      await shell(args[0]);
    } else if (command === "run") {
      await runInTask(args[0], args.slice(1));
    } else if (command === "stop") {
      stopTask(args[0]);
    } else if (command === "rm" || command === "remove") {
      removeTask(args[0]);
    } else if (command === "base" && args[0] === "refresh") {
      baseRefresh(args.slice(1));
    } else if (command === "base" && args[0] === "id") {
      baseId();
    } else {
      console.error(`Unknown command: ${[command, ...args].join(" ")}`);
      console.error(usage());
      process.exit(1);
    }
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exit(1);
  }
}

await main();
