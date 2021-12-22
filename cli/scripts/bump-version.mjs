#!/usr/bin/env node

import shelljs from "shelljs";
import path from "path";
import fs from "fs-extra";
import { fileURLToPath } from "url";
const __dirname = path.dirname(fileURLToPath(import.meta.url));

const file = path.join(__dirname, "../npm/turbo-install/package.json");

const pkg = fs.readJSONSync(file);

const knownWindowsPackages = {
  // "win32 arm64 LE": "turbo-windows-arm64",
  "win32 ia32 LE": "turbo-windows-32",
  "win32 x64 LE": "turbo-windows-64",
};

const knownUnixlikePackages = {
  // "android arm64 LE": "turbo-android-arm64",
  "darwin arm64 LE": "turbo-darwin-arm64",
  "darwin x64 LE": "turbo-darwin-64",
  "freebsd arm64 LE": "turbo-freebsd-arm64",
  "freebsd x64 LE": "turbo-freebsd-64",
  "linux arm LE": "turbo-linux-arm",
  "linux arm64 LE": "turbo-linux-arm64",
  "linux ia32 LE": "turbo-linux-32",
  "linux mips64el LE": "turbo-linux-mips64le",
  "linux ppc64 LE": "turbo-linux-ppc64le",
  // "linux s390x BE": "turbo-linux-s390x",
  "linux x64 LE": "turbo-linux-64",
  // "netbsd x64 LE": "turbo-netbsd-64",
  // "openbsd x64 LE": "turbo-openbsd-64",
  // "sunos x64 LE": "turbo-sunos-64",
};

pkg.optionalDependencies = Object.fromEntries(
  Object.values({
    ...knownWindowsPackages,
    ...knownUnixlikePackages,
  })
    .sort()
    .map((x) => [x, pkg.version])
);

fs.writeFileSync(file, JSON.stringify(pkg, null, 2));
