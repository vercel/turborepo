import fs from "fs";
import fetch from "node-fetch";
import { getCommitDetails } from "./helpers";

const filePath = process.argv[2];
const url = process.argv[3];

const DATA_SOURCE_URL =
  "https://api.us-east.tinybird.co/v0/events?name=turborepo_perf_ttft";

(async function () {
  const contents = fs.readFileSync(filePath);
  const data = JSON.parse(contents.toString());

  const commitDetails = getCommitDetails();

  data.commitSha = commitDetails.commitSha;
  data.commitTimestamp = commitDetails.commitTimestamp;
  data.url = url;

  console.log("Sending data to Tinybird: ", data);

  await fetch(DATA_SOURCE_URL, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${process.env.TINYBIRD_TOKEN}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify(data),
  });
})();
