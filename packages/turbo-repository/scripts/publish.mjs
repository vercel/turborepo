#!/usr/bin/env node

import path from "path";
import { execa } from "execa";
import { copy } from "fs-extra";
import { readFile, writeFile } from "fs/promises";

const cwd = process.cwd();
let platform = process.argv[process.argv.length - 1];

if (platform.endsWith("publish.mjs")) {
  // No platform was passed, default to dev platform
  platform = "darwin-arm64";
}
console.log("platform", platform);

(async function () {
  try {
    // TODO: version
    let version = JSON.parse(
      await readFile(path.join(cwd, "js", "package.json"))
    ).version;

    // Copy binaries to package folders, update version, and publish
    let nativePackagesDir = path.join(cwd, "npm");

    try {
      let binaryName = `repository.${platform}.node`;
      await copy(
        path.join(cwd, "native/@turbo", binaryName),
        path.join(nativePackagesDir, platform, binaryName)
      );
      let pkg = JSON.parse(
        await readFile(path.join(nativePackagesDir, platform, "package.json"))
      );
      pkg.version = version;
      await writeFile(
        path.join(nativePackagesDir, platform, "package.json"),
        JSON.stringify(pkg, null, 2)
      );
      await execa(
        `npm`,
        [`pack`, `${path.join(nativePackagesDir, platform)}`],
        {
          stdio: "inherit",
        }
      );
    } catch (err) {
      // don't block publishing other versions on single platform error
      console.error(`Failed to publish`, platform, err);

      if (
        err.message &&
        err.message.includes(
          "You cannot publish over the previously published versions"
        )
      ) {
        console.error("Ignoring already published error", platform, err);
      } else {
        throw err;
      }
    }
  } catch (err) {
    console.error(err);
    process.exit(1);
  }
})();
