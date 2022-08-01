#!/usr/bin/env node

const fs = require("fs");
const path = require("path");
const exec = require("child_process").exec;

// Get the current version of the package.
const versionFilePath = path.join(__dirname, "..", "version.txt");
const versionFileContents = fs.readFileSync(versionFilePath, "utf-8");
const [currentVersion, currentIdentifier] = versionFileContents.split("\n");

// Check all of the packages to see if they're updated:
const nativeDependencies = [
  "turbo-android-arm64",
  "turbo-darwin-64",
  "turbo-darwin-arm64",
  "turbo-freebsd-64",
  "turbo-freebsd-arm64",
  "turbo-linux-32",
  "turbo-linux-64",
  "turbo-linux-arm",
  "turbo-linux-arm64",
  "turbo-linux-mips64le",
  "turbo-linux-ppc64le",
  "turbo-windows-32",
  "turbo-windows-64",
  "turbo-windows-arm64",
];

function dependencyPublished(dependency, version, identifier, iteration = 0) {
  let delay = Math.pow(2, iteration);
  let resolver, rejecter;
  const awaiter = new Promise((resolve, reject) => {
    resolver = resolve;
    rejecter = reject;
  });

  if (iteration > 4) {
    rejecter(new Error("Too many attempts."));
  }

  setTimeout(() => {
    const command = `npm view ${dependency}@${identifier} version`;
    console.log(`Attempt ${iteration}: ${command}`);
    exec(command, (error, stdout) => {
      if (error) {
        console.error(`exec error: ${error}`);
        process.exit(1);
      }
      if (stdout.trim() === version) {
        resolver();
      } else {
        dependencyPublished(dependency, version, identifier, ++iteration)
          .then(resolver)
          .catch(rejecter);
      }
    });
  }, delay * 1000);

  return awaiter;
}

Promise.all(
  nativeDependencies.map((dependency) =>
    dependencyPublished(dependency, currentVersion, currentIdentifier)
  )
)
  .then(() => {
    console.log("All packages readable on registry. Continuing.");
  })
  .catch((error) => {
    throw new Error(
      `Project lockfile update failed. You must manually update the lockfile. ${error}`
    );
  });
