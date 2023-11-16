import fetch from "node-fetch";
import { type TTFTData, getTTFTData } from "../helpers";

const filePath = process.argv[2];
const runID = process.argv[3];
const token = process.env.TINYBIRD_TOKEN;

if (!token) {
  throw new Error("Missing TINYBIRD_TOKEN env variable");
}

if (!runID) {
  throw new Error("Missing runID");
}

const DATA_SOURCE_NAME = "turborepo_perf_ttft";
const DATA_SOURCE_URL = `https://api.us-east.tinybird.co/v0/events?name=${DATA_SOURCE_NAME}`;

type TinyBirdTTFT = Omit<TTFTData, "cpus">;

async function main() {
  const data = getTTFTData(filePath, runID);

  const { cpus, platform, ...restData } = data;

  const augmentData: TinyBirdTTFT = {
    platform: `${platform}-${cpus}-cores`,
    ...restData,
  };

  console.log("Sending data to Tinybird: ", augmentData);

  const res = await fetch(DATA_SOURCE_URL, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${token}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify(augmentData),
  });

  if (res.ok) {
    console.log("Data sent to Tinybird successfully");
  } else {
    const text = await res.text();
    console.log(text);
  }
}

main()
  .then(() => {
    console.log("done");
  })
  .catch(console.error);
