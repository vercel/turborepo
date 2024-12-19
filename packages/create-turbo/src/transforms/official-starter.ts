import path from "node:path";
import { readJsonSync, writeJsonSync, rmSync, existsSync } from "fs-extra";
import type { PackageJson } from "@turbo/utils";
import { isDefaultExample } from "../utils/isDefaultExample";
import type { TransformInput, TransformResult } from "./types";
import { TransformError } from "./errors";

const meta = {
  name: "official-starter",
};

/**
 * Transform applied to "official starter" examples (those hosted within vercel/turbo/examples)
 **/

// eslint-disable-next-line @typescript-eslint/require-await -- must match transform function signature
export async function transform(args: TransformInput): TransformResult {
  const { prompts, example, opts } = args;

  const defaultExample = isDefaultExample(example.name);
  const isOfficialStarter =
    !example.repo ||
    (example.repo.username === "vercel" && example.repo.name === "turbo");

  if (!isOfficialStarter) {
    return { result: "not-applicable", ...meta };
  }

  // paths
  const rootPackageJsonPath = path.join(prompts.root, "package.json");
  const rootMetaJsonPath = path.join(prompts.root, "meta.json");
  const hasPackageJson = existsSync(rootPackageJsonPath);

  // 1. remove meta file (used for generating the examples page on turbo.build)
  try {
    rmSync(rootMetaJsonPath, { force: true });
  } catch (_err) {
    // do nothing
  }

  if (hasPackageJson) {
    let packageJsonContent;
    try {
      packageJsonContent = readJsonSync(rootPackageJsonPath) as
        | PackageJson
        | undefined;
    } catch {
      throw new TransformError("Unable to read package.json", {
        transform: meta.name,
        fatal: false,
      });
    }

    // if using the basic example, set the name to the project name (legacy behavior)
    if (packageJsonContent) {
      if (defaultExample) {
        packageJsonContent.name = prompts.projectName;
      }

      if (packageJsonContent.devDependencies?.turbo) {
        // if the user specified a turbo version, use that
        if (opts.turboVersion) {
          packageJsonContent.devDependencies.turbo = opts.turboVersion;
          // use the same version as the create-turbo invocation
        } else {
          // eslint-disable-next-line @typescript-eslint/no-var-requires -- Have to go get package.json
          const version = (require("../../package.json") as { version: string })
            .version;
          packageJsonContent.devDependencies.turbo = `^${version}`;
        }
      }

      try {
        writeJsonSync(rootPackageJsonPath, packageJsonContent, {
          spaces: 2,
        });
      } catch (err) {
        throw new TransformError("Unable to write package.json", {
          transform: meta.name,
          fatal: false,
        });
      }
    }
  }

  return { result: "success", ...meta };
}
