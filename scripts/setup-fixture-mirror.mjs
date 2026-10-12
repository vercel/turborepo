#!/usr/bin/env node
// Loopback HTTPS mirror for the standalone release smokes. It serves what
// `turbo setup` resolves and installs for a Node.js + pnpm repository beneath
// the bases that TURBO_SETUP_NODE_MIRROR and TURBO_SETUP_NPM_REGISTRY replace,
// and never contacts a vendor:
//
// - Node.js: the binary running this script, repackaged in the official
//   layout for this host, with a stub bundled npm. Other platforms get
//   placeholder digests, so the lock only installs here.
// - pnpm 10.0.0: a fixture whose `pnpm run <script>` runs the package script
//   through the shell, so tasks run under the managed Node.
//
// Usage: node scripts/setup-fixture-mirror.mjs <state-directory>
// Once listening, it writes "<port> <node-version>" to <state>/ready. Every
// request is appended to <state>/requests.log. While <state>/tamper exists,
// the Node.js archive is served with an extra byte.

import { appendFileSync, existsSync, readFileSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { createServer } from "node:https";
import { join } from "node:path";
import { deflateRawSync, gzipSync } from "node:zlib";

const state = process.argv[2];
if (!state) {
  console.error("Usage: setup-fixture-mirror.mjs <state-directory>");
  process.exit(1);
}
const fixtures = new URL("fixtures/setup-mirror/", import.meta.url);
const version = process.versions.node;
const SPELLINGS = {
  "darwin-x64": "osx-x64-tar",
  "darwin-arm64": "osx-arm64-tar",
  "linux-x64": "linux-x64",
  "linux-arm64": "linux-arm64",
  "win-x64": "win-x64-zip",
  "win-arm64": "win-arm64-zip"
};
const os = { darwin: "darwin", linux: "linux", win32: "win" }[process.platform];
const host = `${os}-${process.arch}`;
if (!(host in SPELLINGS)) {
  throw new Error(`no official Node.js artifact for ${host}`);
}
const NPM = "10.9.0";
const REFUSE = "console.error('fixture npm must not run'); process.exit(97);\n";
const PNPM = `const { spawnSync } = require("node:child_process");
const { readFileSync } = require("node:fs");
const [command, name] = process.argv.slice(2);
console.log("fixture-pnpm 10.0.0 " + process.argv.slice(2).join(" "));
if (command !== "run") process.exit(97);
const script = JSON.parse(readFileSync("package.json", "utf8")).scripts?.[name];
if (!script) process.exit(98);
const result = spawnSync(script, { shell: true, stdio: "inherit" });
process.exit(result.status ?? 1);
`;

// ustar entries without directories: [path, bytes, mode] or [path, link].
function tarGz(entries) {
  const blocks = [];
  for (const [path, body, mode = 0o644] of entries) {
    const link = typeof body === "string";
    const data = link ? Buffer.alloc(0) : body;
    const header = Buffer.alloc(512);
    const field = (offset, length, value) => header.write(value, offset, length);
    const octal = (offset, length, value) =>
      field(offset, length, value.toString(8).padStart(length - 1, "0"));
    field(0, 100, path);
    octal(100, 8, link ? 0o777 : mode);
    octal(108, 8, 0);
    octal(116, 8, 0);
    octal(124, 12, data.length);
    octal(136, 12, 0);
    field(148, 8, "        ");
    field(156, 1, link ? "2" : "0");
    if (link) field(157, 100, body);
    field(257, 8, "ustar\u000000");
    const sum = header.reduce((total, byte) => total + byte, 0);
    field(148, 8, `${sum.toString(8).padStart(6, "0")}\u0000 `);
    blocks.push(header, data, Buffer.alloc((512 - (data.length % 512)) % 512));
  }
  blocks.push(Buffer.alloc(1024));
  return gzipSync(Buffer.concat(blocks));
}

const CRC = Array.from({ length: 256 }, (_, n) => {
  for (let k = 0; k < 8; k++) n = n & 1 ? 0xedb88320 ^ (n >>> 1) : n >>> 1;
  return n >>> 0;
});
function crc32(data) {
  let crc = 0xffffffff;
  for (const byte of data) crc = CRC[(crc ^ byte) & 0xff] ^ (crc >>> 8);
  return (crc ^ 0xffffffff) >>> 0;
}

// Deflated entries, sizes in the local headers (no data descriptors).
function zip(entries) {
  const locals = [];
  const central = [];
  let offset = 0;
  for (const [path, data] of entries) {
    const name = Buffer.from(path);
    const packed = deflateRawSync(data);
    const common = Buffer.alloc(26);
    common.writeUInt16LE(20, 0);
    common.writeUInt16LE(8, 4);
    common.writeUInt16LE(0x21, 8); // 1980-01-01
    common.writeUInt32LE(crc32(data), 10);
    common.writeUInt32LE(packed.length, 14);
    common.writeUInt32LE(data.length, 18);
    common.writeUInt16LE(name.length, 22);
    const local = Buffer.concat([Buffer.from([0x50, 0x4b, 3, 4]), common, name, packed]);
    const record = Buffer.alloc(46);
    record.writeUInt32LE(0x02014b50, 0);
    record.writeUInt16LE(20, 4);
    common.copy(record, 6);
    record.writeUInt32LE(offset, 42);
    central.push(record, name);
    locals.push(local);
    offset += local.length;
  }
  const directory = Buffer.concat(central);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(entries.length, 8);
  end.writeUInt16LE(entries.length, 10);
  end.writeUInt32LE(directory.length, 12);
  end.writeUInt32LE(offset, 16);
  return Buffer.concat([...locals, directory, end]);
}

function nodeArchive(prefix) {
  const node = readFileSync(process.execPath);
  const npm = Buffer.from(
    JSON.stringify({
      name: "npm",
      version: NPM,
      bin: { npm: "bin/npm-cli.js", npx: "bin/npx-cli.js" }
    })
  );
  const refuse = Buffer.from(REFUSE);
  if (os === "win") {
    const cmd = Buffer.from("@echo fixture npm must not run\r\n@exit /b 97\r\n");
    return zip(
      [
        ["node.exe", node],
        ["npm.cmd", cmd],
        ["npx.cmd", cmd],
        ["node_modules/npm/package.json", npm],
        ["node_modules/npm/bin/npm-cli.js", refuse],
        ["node_modules/npm/bin/npx-cli.js", refuse]
      ].map(([path, data]) => [`${prefix}/${path}`, data])
    );
  }
  return tarGz(
    [
      ["bin/node", node, 0o755],
      ["lib/node_modules/npm/package.json", npm],
      ["lib/node_modules/npm/bin/npm-cli.js", refuse, 0o755],
      ["lib/node_modules/npm/bin/npx-cli.js", refuse, 0o755],
      ["bin/npm", "../lib/node_modules/npm/bin/npm-cli.js"],
      ["bin/npx", "../lib/node_modules/npm/bin/npx-cli.js"]
    ].map(([path, ...rest]) => [`${prefix}/${path}`, ...rest])
  );
}

const routes = new Map();
const sums = [];
let archivePath;
for (const spelling of Object.keys(SPELLINGS)) {
  const prefix = `node-v${version}-${spelling}`;
  const file = `${prefix}.${spelling.startsWith("win-") ? "zip" : "tar.gz"}`;
  const bytes = spelling === host ? nodeArchive(prefix) : Buffer.from(file);
  sums.push(`${createHash("sha256").update(bytes).digest("hex")}  ${file}`);
  if (spelling === host) {
    archivePath = `/node/v${version}/${file}`;
    routes.set(archivePath, bytes);
  }
}
routes.set(`/node/v${version}/SHASUMS256.txt`, Buffer.from(`${sums.join("\n")}\n`));
routes.set(
  "/node/index.json",
  Buffer.from(
    JSON.stringify([
      { version: `v${version}`, npm: NPM, lts: false, files: Object.values(SPELLINGS) }
    ])
  )
);
const pnpm = tarGz([
  [
    "package/package.json",
    Buffer.from(
      JSON.stringify({
        name: "pnpm",
        version: "10.0.0",
        bin: { pnpm: "bin/pnpm.cjs", pnpx: "bin/pnpx.cjs" }
      })
    )
  ],
  ["package/bin/pnpm.cjs", Buffer.from(PNPM), 0o755],
  ["package/bin/pnpx.cjs", Buffer.from(REFUSE), 0o755]
]);
const integrity = createHash("sha512").update(pnpm).digest("base64");
routes.set(
  "/npm/pnpm/10.0.0",
  Buffer.from(
    JSON.stringify({
      name: "pnpm",
      version: "10.0.0",
      dist: {
        tarball: "https://registry.npmjs.org/pnpm/-/pnpm-10.0.0.tgz",
        integrity: `sha512-${integrity}`
      }
    })
  )
);
routes.set("/npm/pnpm/-/pnpm-10.0.0.tgz", pnpm);

const log = join(state, "requests.log");
writeFileSync(log, "");
const server = createServer(
  {
    cert: readFileSync(new URL("server.pem", fixtures)),
    key: readFileSync(new URL("server-key.pem", fixtures))
  },
  (request, response) => {
    appendFileSync(log, `${request.method} ${request.url}\n`);
    let body = request.method === "GET" ? routes.get(request.url) : undefined;
    if (body && request.url === archivePath && existsSync(join(state, "tamper"))) {
      body = Buffer.concat([body, Buffer.from([0])]);
    }
    response.writeHead(body ? 200 : 404, { "content-length": body?.length ?? 0 });
    response.end(body);
  }
);
server.listen(0, "127.0.0.1", () => {
  writeFileSync(join(state, "ready"), `${server.address().port} ${version}\n`);
});
