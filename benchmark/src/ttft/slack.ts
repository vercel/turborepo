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

const perPlatform = {
  ubuntu: getTTFTData("profiles/ubuntu-ttft.json", runID),
  macos: getTTFTData("profiles/macos-ttft.json", runID),
  windows: getTTFTData("profiles/windows-ttft.json", runID),
};

// For commitSha and runURL, we use the ubuntu data because it's the same for all platforms
// In the future, we could modify getTTFTData to not include this data and augment it
// ourselves here. This is currently matching ttft-send.ts
const data = {
  commitSha: perPlatform.ubuntu.commitSha,
  runURL: perPlatform.ubuntu.url,
};

for (const platformData of Object.values(perPlatform)) {
  const duration = `${microToSeconds(platformData.durationMicroseconds)}s`;
  data[platformData.platform] = duration;
}

fs.writeFileSync(slackPayloadPath, JSON.stringify(data));

function microToSeconds(micro: number) {
  const milli = micro / 1000;
  const sec = milli / 1000;
  return sec.toFixed(2);
}
