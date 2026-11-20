import * as cp from "node:child_process";
import * as path from "node:path";
import * as fs from "node:fs";

export type Logger = (message: string) => void;

/// Bounded probe time for one `turbo __internal_lsp --probe` invocation.
export const LSP_PROBE_TIMEOUT_MS = 1000;

/// Bounded time for one package-manager discovery command. Package managers
/// can shell out to slow shims, but five seconds is generous for a bin-dir
/// lookup and keeps discovery bounded when they hang.
export const PACKAGE_MANAGER_TIMEOUT_MS = 5000;

export type InternalLspProbeResult =
  | { supported: true }
  | { supported: false; reason: string };

interface ExecCaptureOptions {
  cwd?: string;
  timeout: number;
  signal?: AbortSignal;
}

interface ExecCapture {
  stdout: string;
  stderr: string;
}

/**
 * execFile with a bounded timeout and optional abort signal, capturing
 * stdout/stderr. Never blocks the event loop; on timeout the child is killed
 * and the resulting error carries whatever output was captured.
 */
function execFileCapture(
  command: string,
  args: string[],
  options: ExecCaptureOptions
): Promise<ExecCapture> {
  return new Promise((resolve, reject) => {
    cp.execFile(
      command,
      args,
      {
        cwd: options.cwd,
        timeout: options.timeout,
        signal: options.signal,
        encoding: "utf8",
        // A pathological binary could spew unbounded output before the
        // timeout kills it; cap what we retain.
        maxBuffer: 1024 * 1024
      },
      (error: Error | null, stdout: string, stderr: string) => {
        if (error) {
          reject(Object.assign(error, { stdout, stderr }));
          return;
        }
        resolve({ stdout, stderr });
      }
    );
  });
}

function isAbortError(error: unknown): boolean {
  return (
    error instanceof Error &&
    (error.name === "AbortError" || error.message.includes("aborted"))
  );
}

export function executableNames(name: string) {
  return process.platform === "win32"
    ? [`${name}.exe`, `${name}.cmd`, `${name}`]
    : [name];
}

export function findExecutableInDirectory(
  directory: string,
  name: string
): string | undefined {
  for (const executable of executableNames(name)) {
    const candidate = path.join(directory, executable);
    if (fs.existsSync(candidate) && !fs.statSync(candidate).isDirectory()) {
      return candidate;
    }
  }
}

export function findExecutableOnPath(name: string) {
  const pathEntries = (process.env.PATH ?? "").split(path.delimiter);
  for (const pathEntry of pathEntries) {
    const executable = findExecutableInDirectory(pathEntry, name);
    if (executable && path.basename(executable).startsWith(name)) {
      return executable;
    }
  }
}

export function resolveTurboPath(
  turboPath: string | undefined,
  workspaceRoot: string | undefined,
  log: Logger
) {
  if (!turboPath) {
    return undefined;
  }

  const resolvedPath = path.isAbsolute(turboPath)
    ? turboPath
    : path.resolve(workspaceRoot ?? process.cwd(), turboPath);

  if (!fs.existsSync(resolvedPath)) {
    log(`Manually specified turbo does not exist at path ${turboPath}`);
    return undefined;
  }

  if (fs.statSync(resolvedPath).isDirectory()) {
    return findExecutableInDirectory(resolvedPath, "turbo");
  }

  return resolvedPath;
}

/**
 * Probes a turbo binary for internal LSP support. Async with a bounded
 * timeout, so a slow binary or shim cannot stall the extension host.
 */
export async function probeInternalLsp(
  turboPath: string,
  options: { cwd?: string; signal?: AbortSignal }
): Promise<InternalLspProbeResult> {
  try {
    const { stdout } = await execFileCapture(
      turboPath,
      ["__internal_lsp", "--probe"],
      { ...options, timeout: LSP_PROBE_TIMEOUT_MS }
    );
    const output = stdout.trim();

    if (output === "turbo-lsp") {
      return { supported: true };
    }

    return { supported: false, reason: formatProbeOutput(output) };
  } catch (error) {
    if (isAbortError(error)) {
      throw error;
    }
    return { supported: false, reason: formatProbeError(error) };
  }
}

interface LspCandidate {
  label: string;
  path?: string;
}

/**
 * Finds an installed turbo binary that supports the internal LSP, probing
 * candidates in preference order (configured path, workspace
 * node_modules/.bin, PATH). Candidate paths are deduplicated (resolving
 * symlinks) so the same unsupported binary is not probed twice. Returns
 * undefined when no candidate supports the LSP, in which case the caller
 * falls back to the packaged LSP binary.
 */
export async function findInstalledTurboLsp(options: {
  workspaceRoot?: string;
  configuredTurboPath?: string;
  signal?: AbortSignal;
  log: Logger;
}): Promise<string | undefined> {
  const { workspaceRoot, configuredTurboPath, signal, log } = options;
  log("resolving turbo LSP server");

  const candidates: Array<LspCandidate> = [
    { label: "configured turbo.path", path: configuredTurboPath },
    {
      label: "workspace node_modules/.bin",
      path: workspaceRoot
        ? findExecutableInDirectory(
            path.join(workspaceRoot, "node_modules", ".bin"),
            "turbo"
          )
        : undefined
    },
    { label: "PATH", path: findExecutableOnPath("turbo") }
  ];

  const seen = new Set<string>();
  for (const candidate of candidates) {
    if (!candidate.path) {
      log(`turbo LSP: no candidate from ${candidate.label}`);
      continue;
    }

    // node_modules/.bin entries are usually symlinks; compare real paths so
    // the same binary is never probed twice.
    let dedupeKey = candidate.path;
    try {
      dedupeKey = fs.realpathSync(candidate.path);
    } catch {
      // Keep the literal path if realpath fails.
    }
    if (seen.has(dedupeKey)) {
      log(`turbo LSP: skipping duplicate candidate at ${candidate.path}`);
      continue;
    }
    seen.add(dedupeKey);

    if (signal?.aborted) {
      return undefined;
    }

    log(`turbo LSP: probing ${candidate.label} at ${candidate.path}`);

    const probe = await probeInternalLsp(candidate.path, {
      cwd: workspaceRoot,
      signal
    });
    if (probe.supported) {
      log(`turbo LSP: using ${candidate.label} at ${candidate.path}`);
      return candidate.path;
    }

    log(
      `turbo LSP: rejected ${candidate.label} at ${candidate.path}: ${probe.reason}`
    );
  }

  log(
    "turbo LSP: no installed turbo candidate supports internal LSP; falling back to packaged LSP binary"
  );
  return undefined;
}

interface PackageManagerCheck {
  label: string;
  command: string;
  args: Array<string>;
  interpret: (stdout: string) => string | undefined;
}

/**
 * Finds a workspace-local turbo installation via the available package
 * managers. Each check is async with a bounded timeout, so a slow or hanging
 * package-manager shim cannot stall the extension host.
 */
export async function findLocalTurbo(options: {
  workspaceRoot?: string;
  signal?: AbortSignal;
  log: Logger;
}): Promise<string | undefined> {
  const { workspaceRoot, signal, log } = options;

  if (workspaceRoot) {
    log("attempting to find local turbo in node_modules/.bin");
    const fromNodeModules = findExecutableInDirectory(
      path.join(workspaceRoot, "node_modules", ".bin"),
      "turbo"
    );
    if (fromNodeModules && fs.existsSync(fromNodeModules)) {
      log(`found local turbo at ${fromNodeModules}`);
      return fromNodeModules;
    }
  }

  const checks: Array<PackageManagerCheck> = [
    {
      label: "npm",
      command: "npm",
      args: ["ls", "turbo", "--json"],
      interpret: (stdout) => {
        const npmData = JSON.parse(stdout);
        // this is relative to node_modules
        const packagePath = npmData?.dependencies?.turbo?.resolved;
        const PREFIX = "file:"; // npm ls returns a file: prefix
        if (typeof packagePath === "string" && packagePath.startsWith(PREFIX)) {
          const turboPath = path.join(
            "node_modules",
            packagePath.slice(PREFIX.length),
            "bin",
            "turbo"
          );
          return resolveTurboPath(turboPath, workspaceRoot, log);
        }
        return undefined;
      }
    },
    {
      label: "yarn",
      command: "yarn",
      args: ["bin", "turbo"],
      interpret: (stdout) => resolveTurboPath(stdout.trim(), workspaceRoot, log)
    },
    {
      label: "pnpm",
      command: "pnpm",
      args: ["bin"],
      interpret: (stdout) => findExecutableInDirectory(stdout.trim(), "turbo")
    },
    {
      label: "bun",
      command: "bun",
      args: ["pm", "bin"],
      interpret: (stdout) => findExecutableInDirectory(stdout.trim(), "turbo")
    }
  ];

  for (const check of checks) {
    if (signal?.aborted) {
      return undefined;
    }

    try {
      log(`attempting to find local turbo using ${check.label}`);
      const { stdout } = await execFileCapture(check.command, check.args, {
        cwd: workspaceRoot,
        timeout: PACKAGE_MANAGER_TIMEOUT_MS,
        signal
      });
      const potential = check.interpret(stdout)?.trim();
      if (potential && fs.existsSync(potential)) {
        log(`found local turbo at ${potential}`);
        return potential;
      }
    } catch (error) {
      if (isAbortError(error)) {
        throw error;
      }
      // This package manager is not installed or did not find turbo; try
      // the next one.
    }
  }

  return undefined;
}

/**
 * Finds turbo on PATH, falling back to package-manager discovery. Async so
 * slow package-manager shims never stall the extension host.
 */
export async function findTurbo(options: {
  workspaceRoot?: string;
  signal?: AbortSignal;
  log: Logger;
}): Promise<string | undefined> {
  options.log("attempting to find turbo");
  return findExecutableOnPath("turbo") ?? (await findLocalTurbo(options));
}

function formatProbeOutput(output: string) {
  const line = firstNonEmptyLine(output);
  return line ? `unexpected probe output: ${line}` : "empty probe output";
}

function formatProbeError(error: unknown) {
  const stderr = outputFromError(error, "stderr");
  if (stderr) {
    return firstNonEmptyLine(stderr) ?? "probe command failed with stderr";
  }

  const stdout = outputFromError(error, "stdout");
  if (stdout) {
    return `probe command failed with stdout: ${firstNonEmptyLine(stdout)}`;
  }

  if (error instanceof Error && error.message) {
    return firstNonEmptyLine(error.message) ?? error.message;
  }

  return "probe command failed";
}

function outputFromError(error: unknown, key: "stdout" | "stderr") {
  if (typeof error !== "object" || error === null || !(key in error)) {
    return;
  }

  const output = (error as Record<string, unknown>)[key];
  if (Buffer.isBuffer(output)) {
    return output.toString("utf8").trim();
  }

  if (typeof output === "string") {
    return output.trim();
  }
}

function firstNonEmptyLine(output: string) {
  return output
    .split(/\r?\n/)
    .map((line) => line.trim())
    .find(Boolean);
}
