#!/usr/bin/env node
// Loopback HTTPS mirror for the standalone release smokes. It serves what
// `turbo setup` resolves and installs beneath the bases that the
// TURBO_SETUP_*_MIRROR variables and TURBO_SETUP_NPM_REGISTRY replace, and
// never contacts a vendor:
//
// - /node and /npm: the binary running this script, repackaged in the official
//   Node.js layout for this host with a stub bundled npm (other platforms get
//   placeholder digests, so the lock only installs here), and a fixture
//   pnpm 10.0.0 whose `run` runs package scripts through the shell and whose
//   `install --frozen-lockfile` succeeds without changing anything.
// - /rust, /astral, /golang, and /artifacts, on Linux and macOS: a stand-in
//   rustup that lays out a stable 1.99.0 toolchain, uv 0.13.0 with
//   CPython 3.12.12, Go 1.22.12, and the additional tool smoke-tool 1.2.3.
//   They are shell scripts that answer turbo's probes and the native locked
//   installs and log every call to <state>/calls.log. Windows needs native
//   stand-ins for these (see `posix` below); until then it gets Node.js and
//   pnpm only.
//
// Usage: node scripts/setup-fixture-mirror.mjs <state-directory>
// Once listening, it writes "<port> <node-version>" to <state>/ready. On Linux
// and macOS, <state>/additional-tools.json then holds this host's
// `setup.additionalTools` declaration. Every request is appended to
// <state>/requests.log. While <state>/tamper exists, the Node.js archive is
// served with an extra byte.

import { appendFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { createServer } from "node:https";
import { join, resolve } from "node:path";
import { deflateRawSync, gzipSync } from "node:zlib";

if (!process.argv[2]) {
  console.error("Usage: setup-fixture-mirror.mjs <state-directory>");
  process.exit(1);
}
const state = resolve(process.argv[2]);
const calls = join(state, "calls.log");
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
const PNPM = `const { appendFileSync, readFileSync } = require("node:fs");
const { spawnSync } = require("node:child_process");
const args = process.argv.slice(2);
appendFileSync(${JSON.stringify(calls)}, "pnpm " + args.join(" ") + "\\n");
console.log("fixture-pnpm 10.0.0 " + args.join(" "));
if (args.join(" ") === "install --frozen-lockfile") process.exit(0);
const [command, name] = args;
if (command !== "run") process.exit(97);
const script = JSON.parse(readFileSync("package.json", "utf8")).scripts?.[name];
if (!script) process.exit(98);
const result = spawnSync(script, { shell: true, stdio: "inherit" });
process.exit(result.status ?? 1);
`;

const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");

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
  sums.push(`${sha256(bytes)}  ${file}`);
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

// The stand-ins below are shell scripts. On Windows each one needs a native
// executable with the same behavior; add it here, and the journey's other
// ecosystems can join the Windows smoke.
const posix = os !== "win";
if (posix && state.includes("'")) {
  throw new Error("the state directory must not contain a single quote");
}
// Shell stand-ins log each call, so the journey sees which managed tools
// turbo's probes, the installers, and the tasks ran.
const logCall = (name) => `echo "${name} $*" >> '${calls}'\n`;
const script = (name, body) => Buffer.from(`#!/bin/sh\n${logCall(name)}${body}`);
const cpu = { x64: "x86_64", arm64: "aarch64" }[process.arch];
const triple = os === "darwin" ? `${cpu}-apple-darwin` : `${cpu}-unknown-linux-gnu`;

// Rust: rustup 1.29.1 and the stable channel at 1.99.0 (static.rust-lang.org),
// whose components rustup pins as gzip and xz tarballs.
function rust() {
  const release = "1.99.0";
  const date = "2026-10-01";
  let manifest =
    `manifest-version = "2"\ndate = "${date}"\n[profiles]\n` +
    `minimal = ["rustc", "cargo", "rust-std"]\n[pkg.rust]\nversion = "${release} (b9 ${date})"\n`;
  for (const part of ["rustc", "cargo", "rust-std"]) {
    const url = `https://static.rust-lang.org/dist/${date}/${part}-${release}-${triple}.tar`;
    manifest +=
      `[pkg.${part}]\nversion = "${release}"\n[pkg.${part}.target.${triple}]\navailable = true\n` +
      `url = "${url}.gz"\nhash = "${sha256(`${url}.gz`)}"\n` +
      `xz_url = "${url}.xz"\nxz_hash = "${sha256(`${url}.xz`)}"\n`;
  }
  const cargo = script(
    "cargo",
    `root=$(pwd -P)
id="path+file://$root/crates/rs-app#0.1.0"
case "$1" in
  metadata) printf '{"packages":[{"id":"%s","name":"rs-app","source":null,"manifest_path":"%s/crates/rs-app/Cargo.toml","dependencies":[],"targets":[{"name":"rs_app","kind":["lib"]}]}],"workspace_members":["%s"],"target_directory":"%s/target","metadata":{"name":"rs-root"}}\\n' "$id" "$root" "$id" "$root" ;;
  fetch) [ "$*" = "fetch --locked" ] && [ -n "$CARGO_HOME" ] ;;
  build) echo "managed-cargo $* toolchain=$RUSTUP_TOOLCHAIN secret=\${SMOKE_SECRET:-filtered}" ;;
  *) exit 1 ;;
esac
`
  );
  const rustc = script(
    "rustc",
    `case "$1" in
  -vV) printf 'rustc ${release} (b9 ${date})\\nbinary: rustc\\nhost: ${triple}\\nrelease: ${release}\\n' ;;
  --print) echo ${triple} ;;
  *) exit 1 ;;
esac
`
  );
  const dir = join(state, "rust");
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, "cargo"), cargo, { mode: 0o755 });
  writeFileSync(join(dir, "rustc"), rustc, { mode: 0o755 });
  writeFileSync(join(dir, "manifest.toml"), manifest);
  // rustup verifies and unpacks the channel's components; the stand-in lays
  // out the same toolchain from the files above.
  const rustup = script(
    "rustup",
    `[ "$1 $2" = "toolchain install" ] || exit 1
tc="$RUSTUP_HOME/toolchains/$3-${triple}"
mkdir -p "$tc/bin" "$tc/lib/rustlib" &&
  cp '${dir}/cargo' "$tc/bin/cargo" &&
  cp '${dir}/rustc' "$tc/bin/rustc" &&
  cp '${dir}/rustc' "$tc/bin/rustdoc" &&
  cp '${dir}/manifest.toml' "$tc/lib/rustlib/multirust-channel-manifest.toml" &&
  printf 'cargo-${triple}\\nrust-std-${triple}\\nrustc-${triple}\\n' > "$tc/lib/rustlib/components"
`
  );
  const archive = "/rust/rustup/archive/1.29.1";
  routes.set(`${archive}/${triple}/rustup-init`, rustup);
  for (const target of ["apple-darwin", "unknown-linux-gnu", "unknown-linux-musl", "pc-windows-msvc"]) {
    for (const arch of ["x86_64", "aarch64"]) {
      const exe = target.includes("windows") ? ".exe" : "";
      routes.set(`${archive}/${arch}-${target}/rustup-init${exe}.sha256`, Buffer.from(`${sha256(rustup)}\n`));
    }
  }
  routes.set("/rust/rustup/release-stable.toml", Buffer.from("schema-version = '1'\nversion = '1.29.1'\n"));
  routes.set("/rust/dist/channel-rust-stable.toml", Buffer.from(manifest));
}

// uv 0.13.0 and python-build-standalone CPython 3.12.12 (releases.astral.sh).
function astral() {
  const releases = "/github/python-build-standalone/releases/download/20260211";
  const file = (target) => `cpython-3.12.12%2B20260211-${target}-install_only_stripped.tar.gz`;
  const entry = ([os, arch, libc], target) => ({
    key: `cpython-3.12.12-${os}-${arch}-${libc}`,
    version: "3.12.12",
    os,
    arch,
    libc,
    implementation: "cpython",
    variant: "default",
    url: `https://releases.astral.sh${releases}/${file(target)}`
  });
  const python = script("python", 'echo "Python 3.12.12"\n');
  const archive = tarGz([
    ["python/bin/python3.12", python, 0o755],
    ["python/bin/python3", python, 0o755]
  ]);
  const self = os === "darwin" ? ["macos", cpu, "none"] : ["linux", cpu, "gnu"];
  const listing = [entry(self, triple)];
  const sums = [`${sha256(archive)}  ${file(triple).replace("%2B", "+")}`];
  // Like real `uv python list --all-arches` output, list both Linux
  // architectures; only this host's build is ever downloaded.
  for (const arch of ["x86_64", "aarch64"]) {
    if (self.join() !== ["linux", arch, "gnu"].join()) {
      const target = `${arch}-unknown-linux-gnu`;
      listing.push(entry(["linux", arch, "gnu"], target));
      sums.push(`${"0".repeat(63)}1  ${file(target).replace("%2B", "+")}`);
    }
  }
  routes.set(`/astral${releases}/${file(triple)}`, archive);
  routes.set(`/astral${releases}/SHA256SUMS`, Buffer.from(`${sums.join("\n")}\n`));
  // turbo reads the CPython builds from `uv python list`; its own probes ask
  // for the interpreter.
  const uv = tarGz([
    [
      `uv-${triple}/uv`,
      Buffer.from(`#!/bin/sh
if [ "$1 $2" = "python list" ]; then
  printf '%s\\n' '${JSON.stringify(listing)}'
  exit 0
fi
${logCall("uv")}case "$1 $2" in
  "--version ") echo "uv 0.13.0" ;;
  "python find") command -v python3 ;;
  "sync --locked") ;;
  build*) echo "managed-uv $* python=$UV_PYTHON downloads=$UV_PYTHON_DOWNLOADS secret=\${SMOKE_SECRET:-filtered}" ;;
  *) exit 1 ;;
esac
`),
      0o755
    ],
    [`uv-${triple}/uvx`, Buffer.from("#!/bin/sh\nexit 1\n"), 0o755]
  ]);
  const release = "/astral/github/uv/releases/download/0.13.0";
  routes.set(`${release}/uv-${triple}.tar.gz`, uv);
  routes.set(`${release}/sha256.sum`, Buffer.from(`${sha256(uv)} *uv-${triple}.tar.gz\n`));
  routes.set(
    "/astral/github/versions/main/v1/uv.ndjson",
    Buffer.from(`${JSON.stringify({ version: "0.13.0", artifacts: [] })}\n`)
  );
}

// Go 1.22.12: the go.dev release index and the dl.google.com archive.
function golang() {
  const goos = os === "darwin" ? "darwin" : "linux";
  const goarch = process.arch === "x64" ? "amd64" : "arm64";
  const go = script(
    "go",
    `root=$(pwd -P)
case "$1 $2" in
  "work edit") echo '{"Go":"1.22","Use":[{"DiskPath":"./goapp"}]}' ;;
  "mod edit") echo '{"Module":{"Path":"example.com/goapp"}}' ;;
  "mod graph" | "mod download") ;;
  "list -mod=readonly")
    if [ "$4" = -f ]; then
      echo "$root/goapp/go.mod"
    else
      printf '{"Path":"example.com/goapp","Main":true,"Dir":"%s/goapp"}\\n' "$root"
    fi ;;
  "env -json")
    shift 2
    sep='{'
    for name in "$@"; do
      case "$name" in GOOS) value=${goos} ;; GOCACHE) value="$HOME/go-cache" ;; GOMODCACHE) value="$HOME/go-mod" ;; *) value= ;; esac
      printf '%s"%s":"%s"' "$sep" "$name" "$value"
      sep=,
    done
    echo '}' ;;
  "version ") echo "go version go1.22.12 ${goos}/${goarch}" ;;
  build*)
    [ "$GOROOT/bin/go" -ef "$0" ] && goroot=locked || goroot=other
    echo "managed-go $* GOTOOLCHAIN=$GOTOOLCHAIN goroot=$goroot secret=\${SMOKE_SECRET:-filtered}" ;;
  *) exit 1 ;;
esac
`
  );
  const archive = tarGz([
    ["go/bin/go", go, 0o755],
    ["go/bin/gofmt", script("gofmt", ""), 0o755],
    ["go/src/go.mod", Buffer.from("module std\n")]
  ]);
  const files = [];
  for (const platform of ["darwin-amd64", "darwin-arm64", "linux-amd64", "linux-arm64"]) {
    const filename = `go1.22.12.${platform}.tar.gz`;
    const [system, arch] = platform.split("-");
    const self = platform === `${goos}-${goarch}`;
    if (self) routes.set(`/golang/go/${filename}`, archive);
    files.push({
      filename,
      os: system,
      arch,
      sha256: self ? sha256(archive) : "0".repeat(64),
      version: "go1.22.12",
      size: 1,
      kind: "archive"
    });
  }
  routes.set(
    "/golang/dl/?mode=json&include=all",
    Buffer.from(JSON.stringify([{ version: "go1.22.12", stable: true, files }]))
  );
}

// One root-declared additional tool, served at <mirror>/<host>/<path>.
function additionalTool() {
  const platform = os === "darwin" ? `macos-${process.arch}` : `linux-${process.arch}-gnu`;
  const path = `downloads.example.test/smoke-tool/1.2.3/smoke-tool-1.2.3-${platform}.tar.gz`;
  const archive = tarGz([
    [
      "smoke-tool-1.2.3/bin/smoke-tool",
      script("smoke-tool", 'echo "smoke-tool 1.2.3 secret=${SMOKE_SECRET:-filtered}"\n'),
      0o755
    ],
    ["smoke-tool-1.2.3/share/smoke-tool/README", Buffer.from("smoke-tool fixture\n")]
  ]);
  routes.set(`/artifacts/${path}`, archive);
  const declaration = {
    "smoke-tool": {
      version: "1.2.3",
      artifacts: {
        [platform]: {
          url: `https://${path}`,
          sha256: sha256(archive),
          format: "tar-gz",
          rootPrefix: "smoke-tool-1.2.3",
          executables: { "smoke-tool": "bin/smoke-tool" }
        }
      }
    }
  };
  writeFileSync(join(state, "additional-tools.json"), JSON.stringify(declaration));
}

if (posix) {
  writeFileSync(calls, "");
  rust();
  astral();
  golang();
  additionalTool();
}

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
