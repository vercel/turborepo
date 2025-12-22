import { Readable } from "node:stream";
import { pipeline } from "node:stream/promises";
import type { ReadableStream } from "node:stream/web";
import { createGunzip } from "node:zlib";
import { Parse, type ReadEntry } from "tar";
import { createWriteStream, mkdirSync, rmSync, cpSync } from "node:fs";
import { dirname, resolve, relative, join } from "node:path";
import { execSync } from "node:child_process";

const REQUEST_TIMEOUT = 10000;
const DOWNLOAD_TIMEOUT = 120000;

export interface RepoInfo {
  username: string;
  name: string;
  branch: string;
  filePath: string;
}

export async function isUrlOk(url: string): Promise<boolean> {
  const controller = new AbortController();
  const timeoutId = setTimeout(() => {
    controller.abort();
  }, REQUEST_TIMEOUT);
  try {
    const res = await fetch(url, {
      method: "HEAD",
      signal: controller.signal,
    });
    return res.ok;
  } catch (err) {
    return false;
  } finally {
    clearTimeout(timeoutId);
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
    const controller = new AbortController();
    const timeoutId = setTimeout(() => {
      controller.abort();
    }, REQUEST_TIMEOUT);
    try {
      const infoResponse = await fetch(
        `https://api.github.com/repos/${username}/${name}`,
        { signal: controller.signal }
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
    } catch (err) {
      return;
    } finally {
      clearTimeout(timeoutId);
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
 * @returns true if the path is safe, false if it would escape the root
 */
export function isPathSafe(root: string, strippedPath: string): boolean {
  const resolvedRoot = resolve(root);
  const destPath = resolve(resolvedRoot, strippedPath);
  const relativePath = relative(resolvedRoot, destPath);
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
 * Streaming extraction from a tarball URL.
 */
async function streamingExtract({
  url,
  root,
  strip,
  filter,
}: {
  url: string;
  root: string;
  strip: number;
  filter: (path: string, rootPath: string | null) => boolean;
}) {
  const controller = new AbortController();
  const timeoutId = setTimeout(() => {
    controller.abort();
  }, DOWNLOAD_TIMEOUT);

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
        if (!isPathSafe(root, strippedPath)) {
          console.error(`Blocked path traversal attempt: ${entry.path}`);
          entry.resume();
          return;
        }

        // Block symlinks and hard links to prevent symlink attacks
        if (entry.type && isLinkEntry(entry.type)) {
          console.warn(`Blocked symlink: ${entry.path}`);
          entry.resume();
          return;
        }

        const resolvedRoot = resolve(root);
        const destPath = resolve(resolvedRoot, strippedPath);

        if (entry.type === "Directory") {
          mkdirSync(destPath, { recursive: true });
          entry.resume();
        } else if (entry.type === "File") {
          mkdirSync(dirname(destPath), { recursive: true });
          const writeStream = createWriteStream(destPath);

          // Track when this file write completes
          fileWritePromises.push(
            new Promise<void>((resolve, reject) => {
              writeStream.on("finish", resolve);
              writeStream.on("error", reject);
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

export async function downloadAndExtractExample(root: string, name: string) {
  const tempDir = join(root, ".turbo-clone-temp");

  try {
    // Clone with partial clone (no blobs) and no checkout
    execSync(
      `git clone --filter=blob:none --no-checkout --depth 1 --sparse https://github.com/vercel/turborepo.git "${tempDir}"`,
      { stdio: "pipe" }
    );

    // Set up sparse checkout for just the example we want
    execSync(`git sparse-checkout set examples/${name}`, {
      cwd: tempDir,
      stdio: "pipe",
    });

    // Checkout the files
    execSync("git checkout", {
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
