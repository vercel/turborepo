import fs from "fs";
import { getCommitDetails } from "./helpers";

process.argv.forEach((val, index) => {
  console.log({ index, val });
});

const filePath = process.argv[2];
const runID = process.argv[3];

const contents = fs.readFileSync(filePath);
const data = JSON.parse(contents.toString());

const commitDetails = getCommitDetails();

data.commitSha = commitDetails.commitSha;
data.commitTimestamp = commitDetails.commitTimestamp;
data.runID = runID;

fs.writeFileSync(filePath, JSON.stringify(data, null, 2));
