import path from "path";
import fs from "fs-extra";
import semverPrerelease from "semver/functions/prerelease";
import cliPkgJson from "../../package.json";
import { isDefaultExample } from "../utils/isDefaultExample";
import { TransformInput, TransformResult } from "./types";

// applied to "local" examples (those hosted within vercel/turbo/examples)
export async function transform(args: TransformInput): TransformResult {
  const { prompts, example } = args;

  const defaultExample = isDefaultExample(example.name);
  const isTurboExample =
    !example.repo ||
    (example.repo?.username === "vercel" && example.repo?.name === "turbo");

  if (!isTurboExample) {
    return { result: "not-applicable" };
  }

  // paths
  const rootPackageJsonPath = path.join(prompts.root, "package.json");
  const rootMetaJsonPath = path.join(prompts.root, "meta.json");
  const hasPackageJson = fs.existsSync(rootPackageJsonPath);

  // 1. remove meta file (used for generating the examples page on turbo.build)
  try {
    fs.rmSync(rootMetaJsonPath, { force: true });
  } catch (_err) {}

  if (hasPackageJson) {
    let packageJsonContent;
    try {
      packageJsonContent = fs.readJsonSync(rootPackageJsonPath);
    } catch {
      return { result: "error" };
    }

    // if using the basic example, set the name to the project name (legacy behavior)
    if (packageJsonContent) {
      if (defaultExample) {
        packageJsonContent.name = prompts.projectName;
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
        return { result: "error" };
      }
    }
  }

  return { result: "success" };
}
