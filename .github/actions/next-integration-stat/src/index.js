const { context, getOctokit } = require("@actions/github");
const { info, getInput } = require("@actions/core");
const { default: stripAnsi } = require("strip-ansi");
const { default: fetch } = require("node-fetch");

// A comment marker to identify the comment created by this action.
const BOT_COMMENT_MARKER = `<!-- __marker__ next.js integration stats __marker__ -->`;

// An action report failed next.js integration test with --turbo
async function run() {
  const token = getInput("token");
  const octokit = getOctokit(token);

  const prNumber = context?.payload?.pull_request?.number;
  const prSha = context?.sha;

  console.log("Trying to collect integration stats for PR", {
    prNumber,
    sha: prSha,
  });

  if (!prNumber) {
    info("No PR number found in context, skipping action.");
    return;
  }

  const comments = await octokit.rest.issues.listComments({
    ...context.repo,
    issue_number: prNumber,
  });

  // Get a comment from the bot if it exists
  const existingComment = comments?.data.find(
    (comment) =>
      comment?.user?.login === "github-actions[bot]" &&
      comment?.body?.includes(BOT_COMMENT_MARKER)
  );

  // Iterate all the jobs in the current workflow run
  console.log("Trying to collect next.js integration test logs");
  const jobs = await octokit.paginate(
    octokit.rest.actions.listJobsForWorkflowRun,
    {
      ...context.repo,
      run_id: context?.runId,
      per_page: 50,
    }
  );

  // Filter out next.js integration test jobs
  const integrationTestJobs = jobs?.filter((job) =>
    /Next\.js integration test \([^)]*\)$/.test(job.name)
  );
  console.log(jobs?.map((j) => j.name));

  console.log(
    `Logs found for ${integrationTestJobs.length} jobs`,
    integrationTestJobs.map((job) => job.name)
  );

  const commentTitlePre = `## Failing next.js integration test suites`;
  const commentTitle =
    `${commentTitlePre} ${BOT_COMMENT_MARKER}` + `\nCommit: ${prSha}`;

  let commentToPost = "";
  for (const job of integrationTestJobs) {
    console.log("Checking test results for the job ", job.name);

    // downloadJobLogsForWorkflowRun returns a redirect to the actual logs
    const jobLogRedirectResponse =
      await octokit.rest.actions.downloadJobLogsForWorkflowRun({
        accept: "application/vnd.github+json",
        ...context.repo,
        job_id: job.id,
      });

    // fetch the actual logs
    const jobLogsResponse = await fetch(jobLogRedirectResponse.url, {
      headers: {
        Authorization: `token ${token}`,
      },
    });

    if (!jobLogsResponse.ok) {
      throw new Error(
        `Failed to get logsUrl, got status ${jobLogsResponse.status}`
      );
    }

    // this should be the check_run's raw logs including each line
    // prefixed with a timestamp in format 2020-03-02T18:42:30.8504261Z
    const logText = await jobLogsResponse.text();
    const logs = logText
      .split("\n")
      .map((line) => line.substr("2020-03-02T19:39:16.8832288Z ".length))
      .join("\n");

    if (
      !logs.includes(`failed to pass within`) ||
      !logs.includes("--test output start--")
    ) {
      console.log(`Couldn't find failed tests in logs for job `, job.name);
      continue;
    }

    // Split logs per each test suites, exclude if it's arbitrary log does not contain test data
    const splittedLogs = logs
      .split("NEXT_INTEGRATION_TEST: true")
      .filter((log) => log.includes("--test output start--"));

    // Iterate each chunk of logs, find out test name and corresponding test data
    splittedLogs.forEach((logs) => {
      if (
        !logs.includes(`failed to pass within`) ||
        !logs.includes("--test output start--")
      ) {
        console.log(
          `Couldn't find failed tests in logs, not posting for ${job.name}`
        );
      } else {
        let failedTest = logs.split(`failed to pass within`).shift();

        // Look for the failed test file name
        failedTest = failedTest?.includes("test/")
          ? failedTest?.split("\n").pop()?.trim()
          : "";

        console.log("Failed test: ", { job: job.name, failedTest });

        // Parse JSON-stringified test output between marker
        let testData;
        try {
          testData = logs
            ?.split("--test output start--")
            .pop()
            ?.split("--test output end--")
            ?.shift()
            ?.trim();

          octokit.rest.issues.createComment({
            ...context.repo,
            issue_number: prNumber,
            body: `\`\`\`${JSON.stringify(
              JSON.parse(testData),
              null,
              2
            )}\`\`\``,
          });

          testData = JSON.parse(testData);
        } catch (_) {
          console.log(`Failed to parse test data`);
        }

        const groupedFails = {};
        const testResult = testData.testResults[0];
        const resultMessage = stripAnsi(testResult.message);
        const failedAssertions = testResult.assertionResults.filter(
          (res) => res.status === "failed"
        );

        for (const fail of failedAssertions) {
          const ancestorKey = fail.ancestorTitles.join(" > ");

          if (!groupedFails[ancestorKey]) {
            groupedFails[ancestorKey] = [];
          }
          groupedFails[ancestorKey].push(fail);
        }

        if (existingComment?.body?.includes(prSha)) {
          if (failedTest && existingComment.body?.includes(failedTest)) {
            console.log(
              `Suite is already included in current comment on ${prNumber} `
            );
            // the check_suite comment already says this test failed
            return;
          }
          commentToPost = existingComment.body;
        } else if (!commentToPost || commentToPost.length === 0) {
          commentToPost = `${commentTitle} \n`;
        }

        commentToPost += `\n\`${failedTest}\` `;

        for (const group of Object.keys(groupedFails).sort()) {
          const fails = groupedFails[group];
          commentToPost +=
            `\n- ` +
            fails.map((fail) => `${group} > ${fail.title}`).join("\n- ");
        }

        commentToPost += `\n\n<details>`;
        commentToPost += `\n<summary>Expand output</summary>`;
        commentToPost += `\n\n${resultMessage}`;
        commentToPost += `\n</details>\n`;
      }
    });
  }

  if (!commentToPost || commentToPost.length === 0) {
    console.log("No comment to post, exiting");
    return;
  }

  try {
    if (!existingComment) {
      info("No existing comment found, creating a new one");
      await octokit.rest.issues.createComment({
        ...context.repo,
        issue_number: prNumber,
        body: commentToPost,
      });
      return;
    } else {
      info("Existing comment found, updating it");
      await octokit.rest.issues.updateComment({
        ...context.repo,
        comment_id: existingComment.id,
        body: commentToPost,
      });
      return;
    }
  } catch (error) {
    if (error.status === 403) {
      info(
        "No permission to create a comment. This can happen if PR is created from a fork."
      );
      return;
    }
  }
}

run();
