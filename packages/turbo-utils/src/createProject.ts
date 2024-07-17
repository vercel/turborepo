import path from "node:path";
import retry from "async-retry";
import { dim, red } from "picocolors";
import { mkdir, readJsonSync, existsSync } from "fs-extra";
import * as logger from "./logger";
import {
  downloadAndExtractExample,
  downloadAndExtractRepo,
  getRepoInfo,
  existsInRepo,
  hasRepo,
  type RepoInfo,
} from "./examples";
import { isWriteable } from "./isWriteable";
import { isFolderEmpty } from "./isFolderEmpty";
import type { PackageJson } from "./types";

function isErrorLike(err: unknown): err is { message: string } {
  return (
    typeof err === "object" &&
    err !== null &&
    typeof (err as { message?: unknown }).message === "string"
  );
}

export class DownloadError extends Error {}

export async function createProject({
  appPath,
  example,
  isDefaultExample,
  examplePath,
}: {
  appPath: string;
  example: string;
  isDefaultExample?: boolean;
  examplePath?: string;
}): Promise<{
  cdPath: string;
  hasPackageJson: boolean;
  availableScripts: Array<string>;
  repoInfo?: RepoInfo;
}> {
  let repoInfo: RepoInfo | undefined;
  let repoUrl: URL | undefined;

  if (isDefaultExample) {
    repoInfo = {
      username: "vercel",
      name: "turbo",
      branch: "main",
      filePath: "examples/basic",
    };
  } else {
    try {
      repoUrl = new URL(example);
    } catch (err: unknown) {
      const urlError = err as Error & { code?: string };
      if (urlError.code !== "ERR_INVALID_URL") {
        logger.error(err);
        process.exit(1);
      }
    }

    if (repoUrl) {
      if (repoUrl.origin !== "https://github.com") {
        logger.error(
          `Invalid URL: ${red(
            `"${example}"`
          )}. Only GitHub repositories are supported. Please use a GitHub URL and try again.`
        );
        process.exit(1);
      }

      repoInfo = await getRepoInfo(repoUrl, examplePath);

      if (!repoInfo) {
        logger.error(
          `Unable to fetch repository information from: ${red(
            `"${example}"`
          )}. Please fix the URL and try again.`
        );
        process.exit(1);
      }

      const found = await hasRepo(repoInfo);

      if (!found) {
        logger.error(
          `Could not locate the repository for ${red(
            `"${example}"`
          )}. Please check that the repository exists and try again.`
        );
        process.exit(1);
      }
    } else {
      const found = await existsInRepo(example);

      if (!found) {
        logger.error(
          `Could not locate an example named ${red(
            `"${example}"`
          )}. It could be due to the following:\n`,
          `1. Your spelling of example ${red(
            `"${example}"`
          )} might be incorrect.\n`,
          `2. You might not be connected to the internet or you are behind a proxy.`
        );
        process.exit(1);
      }
    }
  }

  const root = path.resolve(appPath);

  if (!(await isWriteable(path.dirname(root)))) {
    logger.error(
      "The application path is not writable, please check folder permissions and try again."
    );
    logger.error(
      "It is likely you do not have write permissions for this folder."
    );
    process.exit(1);
  }

  const appName = path.basename(root);
  try {
    await mkdir(root, { recursive: true });
  } catch (err) {
    logger.error("Unable to create project directory");
    logger.error(err);
    process.exit(1);
  }
  const { isEmpty, conflicts } = isFolderEmpty(root);
  if (!isEmpty) {
    logger.error(
      `${dim(root)} has ${conflicts.length} conflicting ${
        conflicts.length === 1 ? "file" : "files"
      } - please try a different location`
    );
    process.exit(1);
  }

  const originalDirectory = process.cwd();
  process.chdir(root);

  /**
   * clone the example repository
   */
  logger.log();
  const loader = logger.turboLoader(
    "Downloading files... (This might take a moment)"
  );
  try {
    if (!isDefaultExample && repoInfo) {
      loader.start();
      // eslint-disable-next-line @typescript-eslint/no-non-null-assertion -- this is type guarded above (wtf TS)
      await retry(() => downloadAndExtractRepo(root, repoInfo!), {
        retries: 3,
      });
    } else {
      loader.start();
      await retry(() => downloadAndExtractExample(root, example), {
        retries: 3,
      });
    }
  } catch (reason) {
    throw new DownloadError(
      isErrorLike(reason) ? reason.message : String(reason)
    );
  } finally {
    loader.stop();
  }

  const rootPackageJsonPath = path.join(root, "package.json");
  const hasPackageJson = existsSync(rootPackageJsonPath);
  const availableScripts = [];

  if (hasPackageJson) {
    let packageJsonContent;
    try {
      packageJsonContent = readJsonSync(rootPackageJsonPath) as PackageJson;
    } catch {
      // ignore
    }

    if (packageJsonContent) {
      // read the scripts from the package.json
      availableScripts.push(...Object.keys(packageJsonContent.scripts || {}));
    }
  }

  let cdPath: string = appPath;
  if (path.join(originalDirectory, appName) === appPath) {
    cdPath = appName;
  }

  return { cdPath, hasPackageJson, availableScripts, repoInfo };
}
