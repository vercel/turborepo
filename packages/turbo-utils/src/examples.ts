import { Readable } from "node:stream";
import { pipeline } from "node:stream/promises";
import type { ReadableStream } from "node:stream/web";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { createWriteStream, promises as fs } from "node:fs";
import { x as extract } from "tar";

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

async function downloadTar(url: string, name: string) {
  const tempFile = join(tmpdir(), `${name}.temp-${Date.now()}`);
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
    await pipeline(body, createWriteStream(tempFile));
    return tempFile;
  } finally {
    clearTimeout(timeoutId);
  }
}

export async function downloadAndExtractRepo(
  root: string,
  { username, name, branch, filePath }: RepoInfo
) {
  const tempFile = await downloadTar(
    `https://codeload.github.com/${username}/${name}/tar.gz/${branch}`,
    `turbo-ct-example`
  );

  let rootPath: string | null = null;
  await extract({
    file: tempFile,
    cwd: root,
    strip: filePath ? filePath.split("/").length + 1 : 1,
    filter: (p: string) => {
      // Determine the unpacked root path dynamically instead of hardcoding to the fetched repo's name. This avoids the condition when the repository has been renamed, and the
      // old repository name is used to fetch the example. The tar download will work as it is redirected automatically, but the root directory of the extracted
      // example will be the new, renamed name instead of the name used to fetch the example.
      if (rootPath === null) {
        const pathSegments = p.split("/");
        rootPath = pathSegments.length ? pathSegments[0] : null;
      }
      return p.startsWith(`${rootPath}${filePath ? `/${filePath}/` : "/"}`);
    },
  });

  await fs.unlink(tempFile);
}

export async function downloadAndExtractExample(root: string, name: string) {
  const tempFile = await downloadTar(
    `https://codeload.github.com/vercel/turborepo/tar.gz/main`,
    `turbo-ct-example`
  );

  let rootPath: string | null = null;
  await extract({
    file: tempFile,
    cwd: root,
    strip: 2 + name.split("/").length,
    filter: (p: string) => {
      // Determine the unpacked root path dynamically instead of hardcoding. This avoids the condition when the repository has been renamed, and the
      // old repository name is used to fetch the example. The tar download will work as it is redirected automatically, but the root directory of the extracted
      // example will be the new, renamed name instead of the name used to fetch the example.
      if (rootPath === null) {
        const pathSegments = p.split("/");
        rootPath = pathSegments.length ? pathSegments[0] : null;
      }

      return p.includes(`${rootPath}/examples/${name}/`);
    },
  });

  await fs.unlink(tempFile);
}
