import fs from "node:fs";
import fetch from "node-fetch";
import { getCommitDetails } from "./helpers";

const filePath = process.argv[2];
const runID = process.argv[3];
const token = process.env.TINYBIRD_TOKEN;

if (!token) {
  throw new Error("Missing TINYBIRD_TOKEN env variable");
}

const DATA_SOURCE_URL =
  "https://api.us-east.tinybird.co/v0/events?name=turborepo_perf_ttft";

async function main() {
  const contents = fs.readFileSync(filePath);
  const data = JSON.parse(contents.toString());

  const commitDetails = getCommitDetails();

  data.commitSha = commitDetails.commitSha;
  data.commitTimestamp = commitDetails.commitTimestamp;
  data.url = `https://github.com/vercel/turbo/actions/runs/${runID}`;

  console.log("Sending data to Tinybird: ", data);

  const res = await fetch(DATA_SOURCE_URL, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${token}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify(data),
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
