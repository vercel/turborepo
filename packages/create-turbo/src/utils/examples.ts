import got from "got";
import tar from "tar";
import { Stream } from "stream";
import { promisify } from "util";
import { join } from "path";
import { tmpdir } from "os";
import { createWriteStream, promises as fs } from "fs";

const pipeline = promisify(Stream.pipeline);

export type RepoInfo = {
  username: string;
  name: string;
  branch: string;
  filePath: string;
};

export async function isUrlOk(url: string): Promise<boolean> {
  try {
    const res = await got.head(url);
    return res.statusCode === 200;
  } catch (err) {
    return false;
  }
}

export async function getRepoInfo(
  url: URL,
  examplePath?: string
): Promise<RepoInfo | undefined> {
  const [, username, name, tree, sourceBranch, ...file] =
    url.pathname.split("/");
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
      const infoResponse = await got(
        `https://api.github.com/repos/${username}/${name}`
      );
      const info = JSON.parse(infoResponse.body);
      return { username, name, branch: info["default_branch"], filePath };
    } catch (err) {
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

  return isUrlOk(contentsUrl + packagePath + `?ref=${branch}`);
}

export function existsInRepo(nameOrUrl: string): Promise<boolean> {
  try {
    const url = new URL(nameOrUrl);
    return isUrlOk(url.href);
  } catch {
    return isUrlOk(
      `https://api.github.com/repos/vercel/turbo/contents/examples/${encodeURIComponent(
        nameOrUrl
      )}`
    );
  }
}

async function downloadTar(url: string, name: string) {
  const tempFile = join(tmpdir(), `${name}.temp-${Date.now()}`);
  await pipeline(got.stream(url), createWriteStream(tempFile));
  return tempFile;
}

export async function downloadAndExtractRepo(
  root: string,
  { username, name, branch, filePath }: RepoInfo
) {
  const tempFile = await downloadTar(
    `https://codeload.github.com/${username}/${name}/tar.gz/${branch}`,
    `turbo-ct-example`
  );

  await tar.x({
    file: tempFile,
    cwd: root,
    strip: filePath ? filePath.split("/").length + 1 : 1,
    filter: (p: string) =>
      p.startsWith(
        `${name}-${branch.replace(/\//g, "-")}${
          filePath ? `/${filePath}/` : "/"
        }`
      ),
  });

  await fs.unlink(tempFile);
}

export async function downloadAndExtractExample(root: string, name: string) {
  const tempFile = await downloadTar(
    `https://codeload.github.com/vercel/turbo/tar.gz/main`,
    `turbo-ct-example`
  );

  await tar.x({
    file: tempFile,
    cwd: root,
    strip: 2 + name.split("/").length,
    filter: (p: string) => p.includes(`turbo-main/examples/${name}/`),
  });

  await fs.unlink(tempFile);
}
