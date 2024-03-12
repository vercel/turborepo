import { context, getOctokit } from "@actions/github";
import { setFailed, info } from "@actions/core";
import fs from "node:fs";

const outputDir = process.argv[2];

if (!outputDir) {
  throw new Error("Pass a directory to write the slack payload in");
}
const outputPath = `${outputDir}/slack-payload.json`;

console.log("outputDir: ", outputDir);
console.log("outputPath: ", outputPath);

const NUM_OF_DAYS = 30;
const NUM_OF_ISSUES = 5;

// context.repo is the current repo
const { owner: OWNER, repo: REPO } = context.repo;

// For testing from a fork
// const OWNER = "vercel";
// const REPO = "turbo";

async function run() {
  if (!process.env.GITHUB_TOKEN) throw new TypeError("GITHUB_TOKEN not set");

  try {
    const octoClient = getOctokit(process.env.GITHUB_TOKEN);

    // Get the date (YYYY-MM-DD)
    const date = new Date();
    date.setDate(date.getDate() - NUM_OF_DAYS);
    const daysAgo = date.toISOString().split("T")[0];

    const { data } = await octoClient.rest.search.issuesAndPullRequests({
      order: "desc",
      per_page: NUM_OF_ISSUES,
      q: `repo:${OWNER}/${REPO} is:issue is:open created:>=${daysAgo}`,
      sort: "reactions-+1",
    });

    console.log("Found issues: ", data.items.length);

    if (data.items.length === 0) {
      info("No issues found");
      return;
    }

    const payload = generateWorkflowPayload(data.items);

    fs.writeFileSync(outputPath, JSON.stringify(payload, null, 2));
  } catch (error) {
    console.error(error);
    setFailed(error);
  }
}

function generateWorkflowPayload(issues) {
  const payload = {
    prelude: `Top ${NUM_OF_ISSUES} issues sorted by :+1: reactions (last ${NUM_OF_DAYS} days).*\nNote: This :github2: workflow will run every Monday at 1PM UTC (9AM EST)._"`,
  };

  issues.forEach((issue, index) => {
    payload[`issue${index + 1}URL`] = issue.html_url;

    const count = issue.reactions["+1"];
    payload[`issue${index + 1}Text`] = `:+1: ${count}: ${issue.title}`;
  });

  return payload;
}

run();
