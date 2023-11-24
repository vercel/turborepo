import cp from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { TURBO_BIN, type TTFTData } from "../helpers";
import { run } from "./run";

const profileFile = process.argv[2]; // Should be "windows", "ubuntu" or "macos"

if (!profileFile) {
  console.error("Error: Missing profile name");
  printUsageMessage();
  process.exit(1);
}

if (!profileFile.endsWith(".json")) {
  console.error("Error: please provide a profile name ending in .json");
  printUsageMessage();
  process.exit(1);
}

const profileName = path.basename(profileFile, ".json");

const profileExt = path.extname(profileFile); // Should always be .json, but we'll get it from here anyway.

const ttftFileName = `${profileName}-ttft${profileExt}`;

// process.cwd() should be packages/turbo-benchmark/ here.
const fullProfilePath = path.join(process.cwd(), "profiles", profileFile);
const ttftFilePath = path.join(process.cwd(), "profiles", ttftFileName);

console.log(`Profile will be saved to ${fullProfilePath}`);
console.log(`TTFT data will be saved to ${ttftFilePath}`);

if (fs.existsSync(fullProfilePath)) {
  console.error(`Error: ${fullProfilePath} already exists`);
  printUsageMessage();
  process.exit(1);
}

if (!fs.existsSync(TURBO_BIN)) {
  throw new Error("No turbo binary found");
}

cp.execSync(`${TURBO_BIN} --version`, { stdio: "inherit" });

run(fullProfilePath); // Actual benchmark

interface ProfileItem {
  name: string;
  tid: number;
  pid: number;
  ph: string;
  ts: number;
  ".file"?: string;
  ".line"?: string;
  args?: Record<string, string | number>;
  cat?: string;
  s?: string;
}

type ProfileJSON = Array<ProfileItem>;

const profileJSON = JSON.parse(
  fs.readFileSync(fullProfilePath).toString()
) as ProfileJSON;

const ttftData: TTFTData = {
  name: "time-to-first-task",
  scm: "git",
  platform: "",
  startTimeUnixMicroseconds: 0,
  durationMicroseconds: 0,
  turboVersion: "",
  cpus: 0,
};

// Get the info we need out of the profile
for (const item of profileJSON) {
  if (!item.args) {
    continue;
  }

  const { args } = item;

  if (args.platform) {
    ttftData.platform = `${args.platform}`;
  }

  if (args.numcpus) {
    ttftData.cpus = Number(args.numcpus); // Should always be a number
  }

  if (args.turbo_version) {
    ttftData.turboVersion = `${args.turbo_version}`;
  }

  if (args.start_time) {
    ttftData.startTimeUnixMicroseconds = Number(args.start_time);
  }

  if (args.message === "running visitor") {
    ttftData.durationMicroseconds = item.ts;
  }
}

fs.writeFileSync(ttftFilePath, JSON.stringify(ttftData, null, 2));

// -----------------------
// Helpers
// -----------------------

function printUsageMessage() {
  console.log("Usage:\n\npnpm -F @turbo/benchmark ttft <path>");
}
