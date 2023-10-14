import fs from "fs";
import { getCommitDetails } from "./helpers";

process.argv.forEach((val, index) => {
  console.log({ index, val });
});

const contents = fs.readFileSync("../ttft.json");
const data = JSON.parse(contents.toString());
const commitDetails = getCommitDetails();
data.commitSha = commitDetails.commitSha;
data.commitTimestamp = commitDetails.commitTimestamp;

fs.writeFileSync("../ttft.json", JSON.stringify(data, null, 2));
