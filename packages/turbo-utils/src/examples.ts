import { Readable } from "node:stream";
import { pipeline } from "node:stream/promises";
import type { ReadableStream } from "node:stream/web";
import { createGunzip } from "node:zlib";
import { Parse, type ReadEntry } from "tar";
import { createWriteStream, mkdirSync } from "node:fs";
import { join, dirname } from "node:path";

const REQUEST_TIMEOUT = 10000;
const DOWNLOAD_TIMEOUT = 120000;
const VERCEL_BLOB_BASE_URL =
  "https://ufa25dqjajkmio0q.public.blob.vercel-storage.com";

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

        const destPath = join(root, strippedPath);

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
  await streamingExtract({
    url: `${VERCEL_BLOB_BASE_URL}/examples/${name}.tar.gz`,
    root,
    // The tarball contains a single directory with the example name
    strip: 1,
    filter: () => true,
  });
}
