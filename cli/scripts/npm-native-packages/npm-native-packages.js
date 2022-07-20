#!/usr/bin/env node

const template = require("./template/template.package.json");
const os = process.argv[2];
const arch = process.argv[3];
const version = process.argv[4];

// Map to node os and arch names.

const osLookup = {
  android: "android",
  darwin: "darwin",
  freebsd: "freebsd",
  linux: "linux",
  windows: "win32",
};

const archLookup = {
  32: "ia32",
  64: "x64",
  arm: "arm",
  arm64: "arm64",
  mips64le: "mipsel",
  ppc64le: "ppc64",
};

template.name = `turbo-${os}-${arch}`;
template.description = `The ${os}-${arch} binary for turbo, a monorepo build system.`;
template.os = [osLookup[os]];
template.cpu = [archLookup[arch]];
template.version = version;

console.log(JSON.stringify(template, null, 2));
