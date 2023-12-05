import fs from "node:fs";
import path from "node:path";
import { getTTFTData } from "../helpers";

const runID = process.argv[2];

if (!runID) {
  throw new Error("Missing runID");
}

const slackPayloadPath = path.join(process.cwd(), "slack-payload.json");

console.log("Executing turbo build in child process", {
  cwd: process.cwd(),
  slackPayloadPath,
});

const ubuntu = getTTFTData(
  path.join(process.cwd(), "profiles", "ubuntu-ttft.json"),
  runID
);
const macos = getTTFTData(
  path.join(process.cwd(), "profiles", "macos-ttft.json"),
  runID
);
const windows = getTTFTData(
  path.join(process.cwd(), "profiles", "windows-ttft.json"),
  runID
);

// For commitSha and runURL, we use the ubuntu data because it's the same for all platforms
// In the future, we could modify getTTFTData to not include this data and augment it
// ourselves here. This is currently matching ttft-send.ts
const data = {
  commitSha: ubuntu.commitSha,
  runURL: ubuntu.url,
  ubuntu: `${microToSeconds(ubuntu.durationMicroseconds)}s`,
  windows: `${microToSeconds(windows.durationMicroseconds)}s`,
  macos: `${microToSeconds(macos.durationMicroseconds)}s`,
  "ubuntu-cpus": ubuntu.cpus,
  "windows-cpus": windows.cpus,
  "macos-cpus": macos.cpus,
};

fs.writeFileSync(slackPayloadPath, JSON.stringify(data));

function microToSeconds(micro: number) {
  const milli = micro / 1000;
  const sec = milli / 1000;
  return sec.toFixed(2);
}
