import { Readable, type Writable } from "node:stream";
import { pipeline } from "node:stream/promises";
import type { ReadableStream } from "node:stream/web";
import { createGunzip } from "node:zlib";
import { Parse, type ReadEntry } from "tar";
import { createWriteStream, mkdirSync, rmSync, cpSync } from "node:fs";
import { dirname, resolve, relative, join } from "node:path";
import { execFileSync } from "node:child_process";
import { error, warn } from "./logger";

const REQUEST_TIMEOUT = 10000;
const DOWNLOAD_TIMEOUT = 120000;

/**
 * Performs a fetch request with an automatic timeout.
 * Centralizes the AbortController + setTimeout pattern to avoid repetition.
 */
async function fetchWithTimeout(
  url: string,
  options: RequestInit = {},
  timeoutMs: number = REQUEST_TIMEOUT
): Promise<Response> {
  const controller = new AbortController();
  const timeoutId = setTimeout(() => {
    controller.abort();
  }, timeoutMs);

  try {
    return await fetch(url, {
      ...options,
      signal: controller.signal,
    });
  } finally {
    clearTimeout(timeoutId);
  }
}

export interface RepoInfo {
  username: string;
  name: string;
  branch: string;
  filePath: string;
}

export async function isUrlOk(url: string): Promise<boolean> {
  try {
    const res = await fetchWithTimeout(url, { method: "HEAD" });
    return res.ok;
  } catch {
    return false;
  }
}

export async function getRepoInfo(
  url: URL,
  examplePath?: string
): Promise<RepoInfo | undefined> {
  const [, username, name, tree, sourceBranch, ...file] = url.pathname.split(
    "/"
  ) as Array<string | undefined>;
  const filePath = examplePath
    ? examplePath.replace(/^\//, "")
    : file.join("/");

  if (
    // Support repos whose entire purpose is to be a Turborepo example, e.g.
    // https://github.com/:username/:my-cool-turborepo-example-repo-name.
    tree === undefined ||
    // Support GitHub URL that ends with a trailing slash, e.g.
    // https://github.com/:username/:my-cool-turborepo-example-repo-name/
    // In this case "t" will be an empty string while the turbo part "_branch" will be undefined
    (tree === "" && sourceBranch === undefined)
  ) {
    try {
      const infoResponse = await fetchWithTimeout(
        `https://api.github.com/repos/${username}/${name}`
      );
      if (!infoResponse.ok) {
        return;
      }
      const info = (await infoResponse.json()) as { default_branch: string };
      return {
        username,
        name,
        branch: info.default_branch,
        filePath,
      } as RepoInfo;
    } catch {
      return;
    }
  }

  // If examplePath is available, the branch name takes the entire path
  const branch = examplePath
    ? `${sourceBranch}/${file.join("/")}`.replace(
        new RegExp(`/${filePath}|/$`),
        ""
      )
    : sourceBranch;

  if (username && name && branch && tree === "tree") {
    return { username, name, branch, filePath };
  }
}

export function hasRepo({
  username,
  name,
  branch,
  filePath,
}: RepoInfo): Promise<boolean> {
  const contentsUrl = `https://api.github.com/repos/${username}/${name}/contents`;
  const packagePath = `${filePath ? `/${filePath}` : ""}/package.json`;

  return isUrlOk(`${contentsUrl + packagePath}?ref=${branch}`);
}

export function existsInRepo(nameOrUrl: string): Promise<boolean> {
  try {
    const url = new URL(nameOrUrl);
    return isUrlOk(url.href);
  } catch {
    return isUrlOk(
      `https://api.github.com/repos/vercel/turborepo/contents/examples/${encodeURIComponent(
        nameOrUrl
      )}`
    );
  }
}

/**
 * Validates that a destination path stays within the target root directory.
 * Prevents path traversal attacks (Zip Slip).
 * @param root - The root directory (will be resolved if not already absolute)
 * @param strippedPath - The path to validate
 * @param resolvedRoot - Optional pre-resolved root for performance optimization
 * @returns true if the path is safe, false if it would escape the root
 */
export function isPathSafe(
  root: string,
  strippedPath: string,
  resolvedRoot?: string
): boolean {
  // Check for null bytes which can bypass security checks
  if (strippedPath.includes("\0")) {
    return false;
  }

  // Normalize Windows backslashes to forward slashes before processing
  // This prevents bypasses using mixed path separators
  const normalizedPath = strippedPath.replace(/\\/g, "/");

  // Normalize Unicode to NFC form to prevent normalization bypasses
  // (e.g., combining characters that resolve to "..")
  const unicodeNormalizedPath = normalizedPath.normalize("NFC");

  // Use pre-resolved root if provided, otherwise resolve it
  const rootPath = resolvedRoot ?? resolve(root);
  const destPath = resolve(rootPath, unicodeNormalizedPath);
  const relativePath = relative(rootPath, destPath);
  return !relativePath.startsWith("..") && resolve(destPath) === destPath;
}

/**
 * Checks if a tar entry type is a symlink or hard link.
 * These are blocked to prevent symlink attacks.
 */
export function isLinkEntry(entryType: string): boolean {
  return entryType === "SymbolicLink" || entryType === "Link";
}

/**
 * Options for streaming extraction from a tarball URL.
 */
export interface StreamingExtractOptions {
  url: string;
  root: string;
  strip: number;
  filter: (path: string, rootPath: string | null) => boolean;
}

/**
 * Streaming extraction from a tarball URL.
 * Exported for testing purposes.
 */
export async function streamingExtract({
  url,
  root,
  strip,
  filter,
}: StreamingExtractOptions) {
  const controller = new AbortController();
  const timeoutId = setTimeout(() => {
    controller.abort();
  }, DOWNLOAD_TIMEOUT);

  // Track all write streams so we can clean them up on abort/error
  const writeStreams: Writable[] = [];

  // Pre-resolve root once for performance (avoids calling resolve() per entry)
  const resolvedRoot = resolve(root);

  // Cache created directories to avoid redundant mkdirSync calls
  const createdDirs = new Set<string>();

  try {
    const response = await fetch(url, { signal: controller.signal });
    if (!response.ok || !response.body) {
      throw new Error(`Failed to download: ${response.status}`);
    }

    const body = Readable.fromWeb(response.body as ReadableStream);
    let rootPath: string | null = null;

    // Track all file write operations so we can wait for them to complete
    const fileWritePromises: Array<Promise<void>> = [];

    const parser = new Parse({
      filter: (p: string) => {
        // Determine the unpacked root path dynamically instead of hardcoding.
        // This avoids issues when the repository has been renamed.
        if (rootPath === null) {
          const pathSegments = p.split("/");
          rootPath = pathSegments.length ? pathSegments[0] : null;
        }
        return filter(p, rootPath);
      },
      onentry: (entry: ReadEntry) => {
        // Calculate the stripped path
        const pathParts = entry.path.split("/");
        const strippedPath = pathParts.slice(strip).join("/");

        if (!strippedPath) {
          entry.resume();
          return;
        }

        // Validate the path stays within the target directory (Zip Slip protection)
        // Pass pre-resolved root for performance
        if (!isPathSafe(root, strippedPath, resolvedRoot)) {
          error(`Blocked path traversal attempt: ${entry.path}`);
          entry.resume();
          return;
        }

        // Block symlinks and hard links to prevent symlink attacks
        if (entry.type && isLinkEntry(entry.type)) {
          warn(`Blocked symlink: ${entry.path}`);
          entry.resume();
          return;
        }

        const destPath = resolve(resolvedRoot, strippedPath);

        if (entry.type === "Directory") {
          if (!createdDirs.has(destPath)) {
            mkdirSync(destPath, { recursive: true });
            createdDirs.add(destPath);
          }
          entry.resume();
        } else if (entry.type === "File") {
          const dirPath = dirname(destPath);
          if (!createdDirs.has(dirPath)) {
            mkdirSync(dirPath, { recursive: true });
            createdDirs.add(dirPath);
          }
          const writeStream = createWriteStream(destPath);
          writeStreams.push(writeStream);

          // Track when this file write completes
          fileWritePromises.push(
            new Promise<void>((resolvePromise, rejectPromise) => {
              writeStream.on("finish", resolvePromise);
              writeStream.on("error", rejectPromise);
            })
          );

          entry.pipe(writeStream);
        } else {
          entry.resume();
        }
      },
    });

    await pipeline(body, createGunzip(), parser);

    // Wait for all file writes to complete
    await Promise.all(fileWritePromises);
  } finally {
    clearTimeout(timeoutId);
    // Clean up all write streams to prevent memory leaks on abort/error
    for (const stream of writeStreams) {
      stream.destroy();
    }
  }
}

export async function downloadAndExtractRepo(
  root: string,
  { username, name, branch, filePath }: RepoInfo
) {
  await streamingExtract({
    url: `https://codeload.github.com/${username}/${name}/tar.gz/${branch}`,
    root,
    strip: filePath ? filePath.split("/").length + 1 : 1,
    filter: (p: string, rootPath: string | null) => {
      return p.startsWith(`${rootPath}${filePath ? `/${filePath}/` : "/"}`);
    },
  });
}

/**
 * Validates that an example name contains only safe characters.
 * Prevents command injection and path traversal attacks.
 */
export function isValidExampleName(name: string): boolean {
  return /^[a-zA-Z0-9_-]+$/.test(name) && !name.includes("..");
}

export async function downloadAndExtractExample(root: string, name: string) {
  // Validate example name to prevent command injection
  if (!isValidExampleName(name)) {
    throw new Error(
      `Invalid example name: "${name}". Names must contain only alphanumeric characters, hyphens, and underscores.`
    );
  }

  const tempDir = join(root, ".turbo-clone-temp");

  try {
    // Clone with partial clone (no blobs) and no checkout
    // Using execFileSync with array args to prevent shell injection
    execFileSync(
      "git",
      [
        "clone",
        "--filter=blob:none",
        "--no-checkout",
        "--depth",
        "1",
        "--sparse",
        "https://github.com/vercel/turborepo.git",
        tempDir,
      ],
      { stdio: "pipe" }
    );

    // Set up sparse checkout for just the example we want
    execFileSync("git", ["sparse-checkout", "set", `examples/${name}`], {
      cwd: tempDir,
      stdio: "pipe",
    });

    // Checkout the files
    execFileSync("git", ["checkout"], {
      cwd: tempDir,
      stdio: "pipe",
    });

    // Copy the example files to the root
    const examplePath = join(tempDir, "examples", name);
    cpSync(examplePath, root, { recursive: true });
  } finally {
    // Clean up the temp directory
    rmSync(tempDir, { recursive: true, force: true });
  }
}
