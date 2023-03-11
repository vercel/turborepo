import retry from "async-retry";
import chalk from "chalk";
import fs from "fs-extra";
import path from "path";
import semverPrerelease from "semver/functions/prerelease";

import {
  downloadAndExtractExample,
  downloadAndExtractRepo,
  getRepoInfo,
  existsInRepo,
  hasRepo,
  RepoInfo,
} from "../../utils/examples";
import { addGitIgnore } from "../../utils/git";
import { isFolderEmpty } from "../../utils/isFolderEmpty";
import { isWriteable } from "../../utils/isWriteable";
import { turboLoader, error } from "../../logger";
import cliPkgJson from "../../../package.json";

export class DownloadError extends Error {}

export async function createProject({
  appPath,
  projectName,
  example,
  examplePath,
}: {
  appPath: string;
  projectName: string;
  example: string;
  examplePath?: string;
}): Promise<{
  cdPath: string;
  hasPackageJson: boolean;
  availableScripts: Array<string>;
}> {
  let repoInfo: RepoInfo | undefined;
  let repoUrl: URL | undefined;
  const isDefaultExample = example === "basic" || example === "default";

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
        `Found invalid GitHub URL: ${chalk.red(
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
      const repoInfo2 = repoInfo;
      console.log(
        `\nDownloading files from repo ${chalk.cyan(
          example
        )}. This might take a moment.`
      );
      console.log();
      loader.start();
      await retry(() => downloadAndExtractRepo(root, repoInfo2), {
        retries: 3,
      });
    } else {
      console.log(
        `\nDownloading files${
          !isDefaultExample ? ` for example ${chalk.cyan(example)}` : ""
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
  const rootMetaJsonPath = path.join(root, "meta.json");
  const ignorePath = path.join(root, ".gitignore");
  const hasPackageJson = fs.existsSync(rootPackageJsonPath);
  const isTurboExample =
    repoInfo && repoInfo.username === "vercel" && repoInfo.name === "turbo";
  const availableScripts = [];

  // remove the meta file from turbo examples
  if (isTurboExample || !repoInfo) {
    try {
      fs.rmSync(rootMetaJsonPath, { force: true });
    } catch (err) {
      //  ignore
    }
  }

  if (hasPackageJson) {
    let packageJsonContent;
    try {
      packageJsonContent = fs.readJsonSync(rootPackageJsonPath);
    } catch {
      // ignore
    }

    // if using the basic example, set the name to the project name (legacy behavior)
    if (packageJsonContent) {
      if (isDefaultExample) {
        packageJsonContent.name = projectName;
      }

      // if we're using a pre-release version of create-turbo, install turbo canary instead of latest
      const shouldUsePreRelease = semverPrerelease(cliPkgJson.version) !== null;
      if (shouldUsePreRelease && packageJsonContent?.devDependencies?.turbo) {
        packageJsonContent.devDependencies.turbo = "canary";
      }

      try {
        fs.writeJsonSync(rootPackageJsonPath, packageJsonContent, {
          spaces: 2,
        });
      } catch (err) {
        // ignore
      }

      // read the scripts from the package.json
      availableScripts.push(...Object.keys(packageJsonContent.scripts || {}));
    }
  }

  // Copy `.gitignore` if the application did not provide one
  addGitIgnore(ignorePath);

  let cdPath: string = appPath;
  if (path.join(originalDirectory, appName) === appPath) {
    cdPath = appName;
  }

  return { cdPath, hasPackageJson, availableScripts };
}
