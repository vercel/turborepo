#!/usr/bin/env node

const fs = require("fs");
const path = require("path");
const semver = require("semver");

// These values come from the invocation of release.
const increment = process.argv[2];

// Now we get the current version of the package.
const versionFilePath = path.join(__dirname, "..", "version.txt");
const versionFileContents = fs.readFileSync(versionFilePath, "utf-8");
const [currentVersion] = versionFileContents.split("\n");

// Now that we know current state, figure out what the target state is.
// If we're doing a "pre" release, set the identifier to canary
const identifier = increment.startsWith("pre") ? "canary" : "latest";
const newVersion = semver.inc(currentVersion, increment, identifier);

// Parse the output semver identifer to identify which npm tag to publish to.
const parsed = semver.parse(newVersion);
const tag = parsed?.prerelease[0] || "latest";

fs.writeFileSync(versionFilePath, `${newVersion}\n${tag}\n`);
