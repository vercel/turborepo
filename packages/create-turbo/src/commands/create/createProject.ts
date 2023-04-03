import retry from "async-retry";
import chalk from "chalk";
import fs from "fs-extra";
import path from "path";

import {
  downloadAndExtractExample,
  downloadAndExtractRepo,
  getRepoInfo,
  existsInRepo,
  hasRepo,
  RepoInfo,
} from "../../utils/examples";
import { isFolderEmpty } from "../../utils/isFolderEmpty";
import { isWriteable } from "../../utils/isWriteable";
import { turboLoader, error } from "../../logger";
import { isDefaultExample } from "../../utils/isDefaultExample";

export class DownloadError extends Error {}

export async function createProject({
  appPath,
  example,
  examplePath,
}: {
  appPath: string;
  example: string;
  examplePath?: string;
}): Promise<{
  cdPath: string;
  hasPackageJson: boolean;
  availableScripts: Array<string>;
  repoInfo?: RepoInfo;
}> {
  let repoInfo: RepoInfo | undefined;
  let repoUrl: URL | undefined;
  const defaultExample = isDefaultExample(example);

  try {
    repoUrl = new URL(example);
  } catch (err: any) {
    if (err.code !== "ERR_INVALID_URL") {
      error(err);
      process.exit(1);
    }
  }

  if (repoUrl) {
    if (repoUrl.origin !== "https://github.com") {
      error(
        `Invalid URL: ${chalk.red(
          `"${example}"`
        )}. Only GitHub repositories are supported. Please use a GitHub URL and try again.`
      );
      process.exit(1);
    }

    repoInfo = await getRepoInfo(repoUrl, examplePath);

    if (!repoInfo) {
      error(
        `Unable to fetch repository information from: ${chalk.red(
          `"${example}"`
        )}. Please fix the URL and try again.`
      );
      process.exit(1);
    }

    const found = await hasRepo(repoInfo);

    if (!found) {
      error(
        `Could not locate the repository for ${chalk.red(
          `"${example}"`
        )}. Please check that the repository exists and try again.`
      );
      process.exit(1);
    }
  } else {
    const found = await existsInRepo(example);

    if (!found) {
      error(
        `Could not locate an example named ${chalk.red(
          `"${example}"`
        )}. It could be due to the following:\n`,
        `1. Your spelling of example ${chalk.red(
          `"${example}"`
        )} might be incorrect.\n`,
        `2. You might not be connected to the internet or you are behind a proxy.`
      );
      process.exit(1);
    }
  }

  const root = path.resolve(appPath);

  if (!(await isWriteable(path.dirname(root)))) {
    error(
      "The application path is not writable, please check folder permissions and try again."
    );
    error("It is likely you do not have write permissions for this folder.");
    process.exit(1);
  }

  const appName = path.basename(root);
  try {
    await fs.mkdir(root, { recursive: true });
  } catch (err) {
    error("Unable to create project directory");
    console.error(err);
    process.exit(1);
  }
  const { isEmpty, conflicts } = isFolderEmpty(root);
  if (!isEmpty) {
    error(
      `${chalk.dim(root)} has ${conflicts.length} conflicting ${
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
  const loader = turboLoader("Downloading files...");
  try {
    if (repoInfo) {
      console.log(
        `\nDownloading files from repo ${chalk.cyan(
          example
        )}. This might take a moment.`
      );
      console.log();
      loader.start();
      await retry(() => downloadAndExtractRepo(root, repoInfo as RepoInfo), {
        retries: 3,
      });
    } else {
      console.log(
        `\nDownloading files${
          !defaultExample ? ` for example ${chalk.cyan(example)}` : ""
        }. This might take a moment.`
      );
      console.log();
      loader.start();
      await retry(() => downloadAndExtractExample(root, example), {
        retries: 3,
      });
    }
  } catch (reason) {
    function isErrorLike(err: unknown): err is { message: string } {
      return (
        typeof err === "object" &&
        err !== null &&
        typeof (err as { message?: unknown }).message === "string"
      );
    }
    throw new DownloadError(isErrorLike(reason) ? reason.message : reason + "");
  } finally {
    loader.stop();
  }

  const rootPackageJsonPath = path.join(root, "package.json");
  const hasPackageJson = fs.existsSync(rootPackageJsonPath);
  const availableScripts = [];

  if (hasPackageJson) {
    let packageJsonContent;
    try {
      packageJsonContent = fs.readJsonSync(rootPackageJsonPath);
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
