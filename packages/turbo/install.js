// Most of this file is ripped from esbuild
// @see https://github.com/evanw/esbuild/blob/master/lib/npm/node-install.ts
// This file is MIT licensed.

const nodePlatform = require("./node-platform");
const fs = require("fs");
const path = require("path");
const zlib = require("zlib");
const https = require("https");
const child_process = require("child_process");
const {
  downloadedBinPath,
  TURBO_BINARY_PATH,
  pkgAndSubpathForCurrentPlatform,
} = nodePlatform;
const TURBO_VERSION = require("./package.json").version;

const toPath = path.join(__dirname, "bin", "turbo");
let isToPathJS = true;

function validateBinaryVersion(...command) {
  command.push("--version");
  // Make sure that we get the version of the binary that was just installed
  command.push("--skip-infer");
  const stdout = child_process
    .execFileSync(command.shift(), command, {
      // Without this, this install script strangely crashes with the error
      // "EACCES: permission denied, write" but only on Ubuntu Linux when node is
      // installed from the Snap Store. This is not a problem when you download
      // the official version of node. The problem appears to be that stderr
      // (i.e. file descriptor 2) isn't writable?
      //
      // More info:
      // - https://snapcraft.io/ (what the Snap Store is)
      // - https://nodejs.org/dist/ (download the official version of node)
      // - https://github.com/evanw/esbuild/issues/1711#issuecomment-1027554035
      //
      stdio: "pipe",
    })
    .toString()
    .trim();
  if (stdout !== TURBO_VERSION) {
    throw new Error(
      `Expected ${JSON.stringify(TURBO_VERSION)} but got ${JSON.stringify(
        stdout
      )}`
    );
  }
}

function fetch(url) {
  return new Promise((resolve, reject) => {
    https
      .get(url, (res) => {
        if (
          (res.statusCode === 301 || res.statusCode === 302) &&
          res.headers.location
        )
          return fetch(res.headers.location).then(resolve, reject);
        if (res.statusCode !== 200)
          return reject(new Error(`Server responded with ${res.statusCode}`));
        let chunks = [];
        res.on("data", (chunk) => chunks.push(chunk));
        res.on("end", () => resolve(Buffer.concat(chunks)));
      })
      .on("error", reject);
  });
}

function extractFileFromTarGzip(buffer, subpath) {
  try {
    buffer = zlib.unzipSync(buffer);
  } catch (err) {
    throw new Error(
      `Invalid gzip data in archive: ${(err && err.message) || err}`
    );
  }
  let str = (i, n) =>
    String.fromCharCode(...buffer.subarray(i, i + n)).replace(/\0.*$/, "");
  let offset = 0;
  subpath = `package/${subpath}`;
  while (offset < buffer.length) {
    let name = str(offset, 100);
    let size = parseInt(str(offset + 124, 12), 8);
    offset += 512;
    if (!isNaN(size)) {
      if (name === subpath) return buffer.subarray(offset, offset + size);
      offset += (size + 511) & ~511;
    }
  }
  throw new Error(`Could not find ${JSON.stringify(subpath)} in archive`);
}

function installUsingNPM(pkg, subpath, binPath) {
  // Erase "npm_config_global" so that "npm install --global turbo" works.
  // Otherwise this nested "npm install" will also be global, and the install
  // will deadlock waiting for the global installation lock.
  const env = { ...process.env, npm_config_global: undefined };

  // Create a temporary directory inside the "turbo" package with an empty
  // "package.json" file. We'll use this to run "npm install" in.
  const turboLibDir = path.dirname(require.resolve("turbo"));
  const installDir = path.join(turboLibDir, "npm-install");
  fs.mkdirSync(installDir);
  try {
    fs.writeFileSync(path.join(installDir, "package.json"), "{}");

    // Run "npm install" in the temporary directory which should download the
    // desired package. Try to avoid unnecessary log output. This uses the "npm"
    // command instead of a HTTP request so that it hopefully works in situations
    // where HTTP requests are blocked but the "npm" command still works due to,
    // for example, a custom configured npm registry and special firewall rules.
    child_process.execSync(
      `npm install --loglevel=error --prefer-offline --no-audit --progress=false ${pkg}@${TURBO_VERSION}`,
      { cwd: installDir, stdio: "pipe", env }
    );

    // Move the downloaded binary executable into place. The destination path
    // is the same one that the JavaScript API code uses so it will be able to
    // find the binary executable here later.
    const installedBinPath = path.join(
      installDir,
      "node_modules",
      pkg,
      subpath
    );
    fs.renameSync(installedBinPath, binPath);
  } finally {
    // Try to clean up afterward so we don't unnecessarily waste file system
    // space. Leaving nested "node_modules" directories can also be problematic
    // for certain tools that scan over the file tree and expect it to have a
    // certain structure.
    try {
      removeRecursive(installDir);
    } catch {
      // Removing a file or directory can randomly break on Windows, returning
      // EBUSY for an arbitrary length of time. I think this happens when some
      // other program has that file or directory open (e.g. an anti-virus
      // program). This is fine on Unix because the OS just unlinks the entry
      // but keeps the reference around until it's unused. There's nothing we
      // can do in this case so we just leave the directory there.
    }
  }
}

function removeRecursive(dir) {
  for (const entry of fs.readdirSync(dir)) {
    const entryPath = path.join(dir, entry);
    let stats;
    try {
      stats = fs.lstatSync(entryPath);
    } catch {
      continue; // Guard against https://github.com/nodejs/node/issues/4760
    }
    if (stats.isDirectory()) removeRecursive(entryPath);
    else fs.unlinkSync(entryPath);
  }
  fs.rmSync(dir);
}

async function downloadDirectlyFromNPM(pkg, subpath, binPath) {
  // If that fails, the user could have npm configured incorrectly or could not
  // have npm installed. Try downloading directly from npm as a last resort.
  const url = `https://registry.npmjs.org/${pkg}/-/${pkg}-${TURBO_VERSION}.tgz`;
  console.error(`[turbo] Trying to download ${JSON.stringify(url)}`);
  try {
    fs.writeFileSync(
      binPath,
      extractFileFromTarGzip(await fetch(url), subpath)
    );
    fs.chmodSync(binPath, 0o755);
  } catch (e) {
    console.error(
      `[turbo] Failed to download ${JSON.stringify(url)}: ${
        (e && e.message) || e
      }`
    );
    throw e;
  }
}

async function checkAndPreparePackage() {
  // This feature was added to give external code a way to modify the binary
  // path without modifying the code itself. Do not remove this because
  // external code relies on this (in addition to turbo's own test suite).
  if (TURBO_BINARY_PATH) {
    return;
  }

  const { pkg, subpath } = pkgAndSubpathForCurrentPlatform();

  let binPath;
  try {
    // First check for the binary package from our "optionalDependencies". This
    // package should have been installed alongside this package at install time.
    binPath = require.resolve(`${pkg}/${subpath}`);
  } catch (e) {
    console.error(`[turbo] Failed to find package "${pkg}" on the file system

This can happen if you use the "--no-optional" flag. The "optionalDependencies"
package.json feature is used by turbo to install the correct binary executable
for your current platform. This install script will now attempt to work around
this. If that fails, you need to remove the "--no-optional" flag to use turbo.
`);

    // If that didn't work, then someone probably installed turbo with the
    // "--no-optional" flag. Attempt to compensate for this by downloading the
    // package using a nested call to "npm" instead.
    //
    // THIS MAY NOT WORK. Package installation uses "optionalDependencies" for
    // a reason: manually downloading the package has a lot of obscure edge
    // cases that fail because people have customized their environment in
    // some strange way that breaks downloading. This code path is just here
    // to be helpful but it's not the supported way of installing turbo.
    binPath = downloadedBinPath(pkg, subpath);
    try {
      console.error(`[turbo] Trying to install package "${pkg}" using npm`);
      installUsingNPM(pkg, subpath, binPath);
    } catch (e2) {
      console.error(
        `[turbo] Failed to install package "${pkg}" using npm: ${
          (e2 && e2.message) || e2
        }`
      );

      // If that didn't also work, then something is likely wrong with the "npm"
      // command. Attempt to compensate for this by manually downloading the
      // package from the npm registry over HTTP as a last resort.
      try {
        await downloadDirectlyFromNPM(pkg, subpath, binPath);
      } catch (e3) {
        throw new Error(`Failed to install package "${pkg}"`);
      }
    }
  }
}

checkAndPreparePackage().then(() => {
  try {
    if (isToPathJS) {
      // We need "node" before this command since it's a JavaScript file
      validateBinaryVersion("node", toPath);
    } else {
      // This is no longer a JavaScript file so don't run it using "node"
      validateBinaryVersion(toPath);
    }
  } catch (err) {
    if (
      process.platform === "linux" &&
      err.message &&
      err.message.includes("ENOENT")
    ) {
      console.error(
        `Error: Failed to run turbo binary, you may need to install glibc compat\nSee https://turbo.build/repo/docs/getting-started/existing-monorepo#install-turbo`
      );
    }
    throw err;
  }
});
