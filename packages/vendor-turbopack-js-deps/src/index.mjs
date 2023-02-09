/**
 * Build npm package to be able to embed them in the binary
 */

import { mkdir, writeFile, rm, readFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { createRequire } from "node:module";
import { join, dirname, basename, extname } from "node:path";
import { pathToFileURL } from "node:url";

import ncc from "@vercel/ncc";
import { findUp } from "find-up";

const cwd = process.cwd();
const require = createRequire(pathToFileURL(join(cwd, "index.js")));

// adapted from https://github.com/vercel/next.js/blob/8fb5ef18e7958a19874e11b8037ac0f71c48baef/packages/next/taskfile-ncc.js
async function writePackageManifest(packageName, main) {
  // some newer packages fail to include package.json in the exports
  // so we can't reliably use require.resolve here
  let packagePath;

  try {
    packagePath = require.resolve(packageName + "/package.json");
  } catch (_) {
    packagePath = await findUp("package.json", {
      cwd: dirname(require.resolve(packageName)),
    });
  }
  const { name, author, license, version } = require(packagePath);

  const compiledPackagePath = join(cwd, `src/compiled/${packageName}`);

  const potentialLicensePath = join(dirname(packagePath), "./LICENSE");
  if (existsSync(potentialLicensePath)) {
    await writeFile(
      join(compiledPackagePath, "LICENSE"),
      await readFile(potentialLicensePath, "utf8")
    );
  } else {
    // license might be lower case and not able to be found on case-sensitive
    // file systems (ubuntu)
    const otherPotentialLicensePath = join(dirname(packagePath), "./license");
    if (existsSync(otherPotentialLicensePath)) {
      await writeFile(
        join(compiledPackagePath, "LICENSE"),
        await readFile(otherPotentialLicensePath, "utf8")
      );
    }
  }

  await writeFile(
    join(compiledPackagePath, "package.json"),
    JSON.stringify(
      Object.assign(
        {},
        {
          name,
          main: `${basename(main)}`,
          types: `${basename(main, extname(main))}.d.ts`,
        },
        author ? { author } : undefined,
        license ? { license } : undefined,
        version ? { version } : undefined
      ),
      null,
      2
    ) + "\n"
  );
}

/**
 *
 * @param {{ name: string; type: "cjs" | "module" | "module-default"; types?: string; }} pkg
 * @param {string} main
 * @returns {Promise<void>}
 */
async function writeTypes(pkg, main) {
  let types = "";
  if (pkg.types) {
    types += `${pkg.types}\n`;
  } else if (pkg.type === "module-default") {
    types += `import m from "${pkg.name}";\n`;
    types += `export default m;\n`;
  } else if (pkg.type === "module") {
    types += `export * from "${pkg.name}";\n`;
  } else if (pkg.type === "cjs") {
    types += `import m from "${pkg.name}";\n`;
    types += `export = m;\n`;
  } else {
    throw new Error(`unknown package type ${pkg.type} for ${pkg.name}`);
  }

  const compiledPackagePath = join(cwd, `src/compiled/${pkg.name}`);

  await writeFile(
    join(compiledPackagePath, `${basename(main, extname(main))}.d.ts`),
    types
  );
}

async function main() {
  const baseDir = join(cwd, "src/compiled");

  await rm(baseDir, {
    force: true,
    recursive: true,
  });

  let packageJSON = {};
  try {
    const content = await readFile(join(cwd, "package.json"));
    packageJSON = JSON.parse(content.toString("utf-8"));
  } catch (e) {
    console.error("failed to read package.json");
    throw e;
  }

  if (!packageJSON.vendoredDependencies) {
    console.log(
      "package.json does not contain a `vendoredDependencies` object"
    );
    return;
  }

  /**
   * @type {{
   *   name: string;
   *   type: "cjs" | "module" | "module-default";
   *   types?: string;
   * }[]}
   */
  const packages = Object.entries(packageJSON.vendoredDependencies).map(
    ([name, obj]) => ({
      ...obj,
      name,
    })
  );
  const externals = Object.fromEntries(
    packages.map((pkg) => [
      pkg.name,
      `${packageJSON.name}/compiled/${pkg.name}`,
    ])
  );

  for (const pkg of packages) {
    const input = require.resolve(pkg.name);

    const outputDir = join(baseDir, pkg.name);
    await mkdir(outputDir, { recursive: true });

    const { code, assets } = await ncc(input, {
      minify: true,
      assetBuilds: true,
      quiet: true,
      externals,
    });

    const mainFile = "index.js";

    await writeFile(join(outputDir, mainFile), code);

    for (const key in assets) {
      await writeFile(join(outputDir, key), assets[key].source);
    }

    await writePackageManifest(pkg.name, mainFile);
    await writeTypes(pkg, mainFile);

    console.log(`built ${pkg.name}`);
  }
}

main().catch((e) => {
  console.dir(e);
  process.exit(1);
});
