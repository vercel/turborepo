// @ts-check
import { context, getOctokit } from "@actions/github";
import { setFailed, info } from "@actions/core";
import fs from "node:fs";

const dirToWriteSlackPayloadIn = process.argv[2];

if (!dirToWriteSlackPayloadIn) {
  throw new Error("Pass a directory to write the slack payload in");
}

console.log("dirToWriteSlackPayloadIn: ", dirToWriteSlackPayloadIn);
const fileToWriteSlackPayloadIn = `${dirToWriteSlackPayloadIn}/slack-payload.json`;

function generateBlocks(issues) {
  const prelude =
    "*A list of the top 15 issues sorted by most :+1: reactions over the last 90 days.*\n_Note: This :github2: workflow will run every Monday at 1PM UTC (9AM EST)._";

  // Slack Markup language
  // <https://www.google.com|Click here to visit Google>

  const lines = issues.map((issue, i) => {
    const url = issue.html_url;
    const number = issue.number;
    const link = `<${url}|${number}>`;
    const count = issue.reactions["+1"];
    const line = `${i + 1}. [${link}, :+1: ${count}]: ${issue.title}`;
    return line;
  });

  return [prelude, ...lines].join("\n");
}

async function run() {
  try {
    if (!process.env.GITHUB_TOKEN) throw new TypeError("GITHUB_TOKEN not set");

    const octoClient = getOctokit(process.env.GITHUB_TOKEN);

    // Get the date 90 days ago (YYYY-MM-DD)
    const date = new Date();
    date.setDate(date.getDate() - 90);
    const ninetyDaysAgo = date.toISOString().split("T")[0];

    // const { owner, repo } = context.repo;
    const owner = "vercel";
    const repo = "turbo";
    const { data } = await octoClient.rest.search.issuesAndPullRequests({
      order: "desc",
      per_page: 15,
      q: `repo:${owner}/${repo} is:issue is:open created:>=${ninetyDaysAgo}`,
      sort: "reactions-+1",
    });

    console.log("Found issues: ", data.items.length);

    if (data.items.length === 0) {
      info("No issues found");
      return;
    }

    const text = generateBlocks(data.items);
    fs.writeFileSync(
      fileToWriteSlackPayloadIn,
      JSON.stringify({ text }, null, 2)
    );
  } catch (error) {
    setFailed(error);
  }
}

run();
