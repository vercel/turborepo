import { context, getOctokit } from "@actions/github";
import { info, getInput } from "@actions/core";
const { default: stripAnsi } = require("strip-ansi");
const { default: nodeFetch } = require("node-fetch");
const fs = require("fs");
const semver = require("semver");

/**
 * Models parsed test results output from next.js integration test.
 * This is a subset of the full test result output from jest, partially compatible.
 */
interface TestResult {
  numFailedTestSuites: number;
  numFailedTests: number;
  numPassedTestSuites: number;
  numPassedTests: number;
  numPendingTestSuites: number;
  numPendingTests: number;
  numRuntimeErrorTestSuites: number;
  numTodoTests: number;
  numTotalTestSuites: number;
  numTotalTests: number;
  startTime: number;
  success: boolean;
  testResults?: Array<{
    assertionResults?: Array<{
      ancestorTitles?: Array<string> | null;
      failureMessages?: Array<string> | null;
      fullName: string;
      location?: null;
      status: string;
      title: string;
    }> | null;
    endTime: number;
    message: string;
    name: string;
    startTime: number;
    status: string;
    summary: string;
  }> | null;
  wasInterrupted: boolean;
}

type Octokit = ReturnType<typeof getOctokit>;

type Job = Awaited<
  ReturnType<Octokit["rest"]["actions"]["listJobsForWorkflowRun"]>
>["data"]["jobs"][number];

type ExistingComment =
  | Awaited<
      ReturnType<Octokit["rest"]["issues"]["listComments"]>
    >["data"][number]
  | undefined;
interface FailedJobResult {
  job: string;
  /**
   * Failed test file name
   */
  name: string;
  data: TestResult;
}
interface TestResultManifest {
  nextjsVersion: string;
  ref: string;
  result: Array<FailedJobResult>;
}

// A comment marker to identify the comment created by this action.
const BOT_COMMENT_MARKER = `<!-- __marker__ next.js integration stats __marker__ -->`;
// Header for the test report.
const commentTitlePre = `## Failing next.js integration test suites`;

// Download logs for a job in a workflow run by reading redirect url from workflow log response.
async function fetchJobLogsFromWorkflow(
  octokit: Octokit,
  token: string,
  job: Job
): Promise<{ nextjsVersion: string; logs: string; job: Job }> {
  console.log("Checking test results for the job ", job.name);

  // downloadJobLogsForWorkflowRun returns a redirect to the actual logs
  const jobLogRedirectResponse =
    await octokit.rest.actions.downloadJobLogsForWorkflowRun({
      accept: "application/vnd.github+json",
      ...context.repo,
      job_id: job.id,
    });

  // fetch the actual logs
  const jobLogsResponse = await nodeFetch(jobLogRedirectResponse.url, {
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
  const logText: string = await jobLogsResponse.text();
  const dateTimeStripped = logText
    .split("\n")
    .map((line) => line.substr("2020-03-02T19:39:16.8832288Z ".length));

  const nextjsVersion = dateTimeStripped
    .find((x) => x.includes("RUNNING NEXTJS VERSION:") && !x.includes("$("))
    ?.split("RUNNING NEXTJS VERSION:")
    .pop()
    ?.trim()!;
  const logs = dateTimeStripped.join("\n");

  return { nextjsVersion, logs, job };
}

// Filter out logs that does not contain failed tests, then parse test results into json
function collectFailedTestResults(
  splittedLogs: Array<string>,
  job: Job
): Array<FailedJobResult> {
  return splittedLogs
    .filter((logs) => {
      if (
        !logs.includes(`failed to pass within`) ||
        !logs.includes("--test output start--")
      ) {
        console.log(
          `Couldn't find failed tests in logs, not posting for ${job.name}`
        );
        return false;
      }
      return true;
    })
    .map((logs) => {
      let failedTest = logs.split(`failed to pass within`).shift();

      // Look for the failed test file name
      failedTest = failedTest?.includes("test/")
        ? failedTest?.split("\n").pop()?.trim()
        : "";

      console.log("Failed test: ", { job: job.name, failedTest });

      // Parse JSON-stringified test output between marker
      try {
        const testData = logs
          ?.split("--test output start--")
          .pop()
          ?.split("--test output end--")
          ?.shift()
          ?.trim()!;

        return {
          job: job.name,
          name: failedTest,
          data: JSON.parse(testData),
        };
      } catch (_) {
        console.log(`Failed to parse test data`);
        return null;
      }
    })
    .filter(Boolean) as Array<FailedJobResult>;
}

// Collect necessary inputs to run actions,
async function getInputs(): Promise<{
  token: string;
  shouldDiffWithMain: boolean;
  octokit: Octokit;
  prNumber: number | undefined;
  sha: string;
  existingComment: ExistingComment;
}> {
  const token = getInput("token");
  const shouldDiffWithMain = getInput("diff_base") === "main";
  if (getInput("diff_base") !== "main" && getInput("diff_base") !== "release") {
    console.error('Invalid diff_base, must be "main" or "release"');
    process.exit(1);
  }

  const octokit = getOctokit(token);

  const prNumber = context?.payload?.pull_request?.number;
  const sha = context?.sha;

  let comments: Awaited<
    ReturnType<typeof octokit.rest.issues.listComments>
  > | null = null;
  let existingComment: ExistingComment;

  if (prNumber) {
    console.log("Trying to collect integration stats for PR", {
      prNumber,
      sha: sha,
    });

    comments = await octokit.rest.issues.listComments({
      ...context.repo,
      issue_number: prNumber,
    });

    // Get a comment from the bot if it exists
    existingComment = comments?.data.find(
      (comment) =>
        comment?.user?.login === "github-actions[bot]" &&
        comment?.body?.includes(BOT_COMMENT_MARKER)
    );
  } else {
    info("No PR number found in context, will not try to post comment.");
  }

  return {
    token,
    shouldDiffWithMain,
    octokit,
    prNumber,
    sha,
    existingComment,
  };
}

// Iterate all the jobs in the current workflow run, collect & parse logs for failed jobs for the postprocessing.
async function getFailedJobResults(
  octokit: Octokit,
  token: string,
  sha: string
): Promise<TestResultManifest> {
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

  // Iterate over all of next.js integration test jobs, read logs and collect failed test results if exists.
  const fullJobLogsFromWorkflow = await Promise.all(
    integrationTestJobs.map((job) =>
      fetchJobLogsFromWorkflow(octokit, token, job)
    )
  );

  const testResultManifest: TestResultManifest = {
    ref: sha,
  } as any;

  const failedJobResults = fullJobLogsFromWorkflow
    .filter(({ logs, job }) => {
      if (
        !logs.includes(`failed to pass within`) ||
        !logs.includes("--test output start--")
      ) {
        console.log(`Couldn't find failed tests in logs for job `, job.name);
        return false;
      }
      return true;
    })
    .reduce((acc, { logs, nextjsVersion, job }) => {
      testResultManifest.nextjsVersion = nextjsVersion;

      // Split logs per each test suites, exclude if it's arbitrary log does not contain test data
      const splittedLogs = logs
        .split("NEXT_INTEGRATION_TEST: true")
        .filter((log) => log.includes("--test output start--"));

      // Iterate each chunk of logs, find out test name and corresponding test data
      const failedTestResultsData = collectFailedTestResults(splittedLogs, job);

      return acc.concat(failedTestResultsData);
    }, [] as Array<FailedJobResult>);

  testResultManifest.result = failedJobResults;

  // Collect all test results into single manifest to store into file. This'll allow to upload / compare test results
  // across different runs.
  fs.writeFileSync(
    "./nextjs-test-results.json",
    JSON.stringify(testResultManifest, null, 2)
  );

  return testResultManifest;
}

// Get the latest base test results to diff against with current test results.
async function getTestResultDiffBase(
  octokit: Octokit,
  shouldDiffWithMain: boolean
): Promise<TestResultManifest | null> {
  console.log("Trying to find latest test results to compare");

  // First, get the tree of `test-results` from `nextjs-integration-test-data` branch
  const branchTree = (
    await octokit.rest.git.getTree({
      ...context.repo,
      tree_sha: "refs/heads/nextjs-integration-test-data",
    })
  ).data.tree.find((tree) => tree.path === "test-results");

  if (!branchTree || !branchTree.sha) {
    console.error("Couldn't find existing test results");
    return null;
  }

  // Get the trees under `/test-results`
  const testResultsTree = (
    await octokit.rest.git.getTree({
      ...context.repo,
      tree_sha: branchTree.sha,
    })
  ).data.tree;

  // If base is main, get the tree under `test-results/main`
  // Otherwise iterate over all the trees under `test-results` then find latest next.js release
  let baseTree:
    | Awaited<
        ReturnType<Awaited<Octokit["rest"]["git"]["getTree"]>>
      >["data"]["tree"][number]
    | undefined;
  if (shouldDiffWithMain) {
    console.log("Trying to find latest test results from main branch");
    baseTree = testResultsTree.find((tree) => tree.path === "main");
  } else {
    console.log("Trying to find latest test results from next.js release");
    baseTree = testResultsTree
      .filter((tree) => tree.path !== "main")
      .reduce((acc, value) => {
        if (!acc) {
          return value;
        }

        return semver.gt(value.path, acc.path) ? value : acc;
      }, null as any as typeof baseTree);
  }

  if (!baseTree || !baseTree.sha) {
    console.log("There is no base to compare test results against");
    return null;
  }

  console.log("Found base tree", baseTree);

  // Now tree should point the list of .json for the actual test results
  const testResultJsonTree = (
    await octokit.rest.git.getTree({
      ...context.repo,
      tree_sha: baseTree.sha,
    })
  ).data.tree;

  if (!testResultJsonTree) {
    console.log("There is no test results stored in the base yet");
    return null;
  }

  // Find the latest test result tree, iterate results file names to find out the latest one.
  // Filename follow ${yyyyMMddHHmm}-${sha}.json format.
  const actualTestResultTree = testResultJsonTree.reduce((acc, value) => {
    const dateStr = value.path?.split("-")[0].match(/(....)(..)(..)(..)(..)/);

    const date = new Date(
      dateStr![1] as any,
      (dateStr![2] as any) - 1,
      dateStr![3] as any,
      dateStr![4] as any,
      dateStr![5] as any
    );
    if (!acc) {
      return {
        date,
        value,
      };
    }

    return acc.date >= date ? acc : { date, value };
  }, null as any as { date: Date; value: typeof testResultJsonTree[0] });

  if (!actualTestResultTree || !actualTestResultTree?.value?.sha) {
    console.log("There is no test results json stored in the base yet");
    return null;
  }

  console.log(
    "Found test results to compare against: ",
    actualTestResultTree.value
  );

  // actualTestResultTree should point to the file that contains the test results
  // we can try to read now.
  const { data } = await octokit.rest.git.getBlob({
    ...context.repo,
    file_sha: actualTestResultTree.value.sha,
  });

  const { encoding, content } = data;

  if (encoding === "base64") {
    return JSON.parse(Buffer.from(content, "base64").toString());
  } else if (encoding === "utf-8") {
    return JSON.parse(content);
  } else {
    throw new Error("Unknown encoding: " + encoding);
  }
}

function getTestSummary(
  sha: string,
  shouldDiffWithMain: boolean,
  baseResults: TestResultManifest | null,
  failedJobResults: TestResultManifest,
  shouldShareTestSummaryToSlack: boolean
) {
  // Read current tests summary
  const {
    currentTestFailedSuiteCount,
    currentTestFailedCaseCount,
    currentTestFailedNames,
  } = failedJobResults.result.reduce(
    (acc, value) => {
      const { data, name } = value;
      acc.currentTestFailedSuiteCount += data.numFailedTestSuites;
      acc.currentTestFailedCaseCount += data.numFailedTests;
      acc.currentTestFailedNames.push(name);

      return acc;
    },
    {
      currentTestFailedSuiteCount: 0,
      currentTestFailedCaseCount: 0,
      currentTestFailedNames: [] as Array<string>,
    }
  );

  console.log("Current test summary", {
    currentTestFailedCaseCount,
    currentTestFailedSuiteCount,
    currentTestFailedNames,
  });

  if (!baseResults) {
    console.log("There's no base to compare");

    return `### Test summary
|   | Current (${sha}) | Diff |
|---|---|---|
| Failed Suites | ${currentTestFailedSuiteCount} | N/A |
| Failed Cases | ${currentTestFailedCaseCount} | N/A |`;
  }

  const {
    baseTestFailedSuiteCount,
    baseTestFailedCaseCount,
    baseTestFailedNames,
  } = baseResults.result.reduce(
    (acc, value) => {
      const { data, name } = value;
      acc.baseTestFailedSuiteCount += data.numFailedTestSuites;
      acc.baseTestFailedCaseCount += data.numFailedTests;
      acc.baseTestFailedNames.push(name);
      return acc;
    },
    {
      baseTestFailedSuiteCount: 0,
      baseTestFailedCaseCount: 0,
      baseTestFailedNames: [] as Array<string>,
    }
  );

  console.log("Base test summary", {
    baseTestFailedSuiteCount,
    baseTestFailedCaseCount,
    baseTestFailedNames,
  });

  let testSuiteDiff = ":zero:";
  const suiteCountDiff = baseTestFailedSuiteCount - currentTestFailedSuiteCount;
  if (suiteCountDiff > 0) {
    testSuiteDiff = `:arrow_down_small: ${suiteCountDiff}`;
  } else if (suiteCountDiff < 0) {
    testSuiteDiff = `:arrow_up_small: ${-suiteCountDiff}`;
  }

  let testCaseDiff = ":zero:";
  const caseCountDiff = baseTestFailedCaseCount - currentTestFailedCaseCount;
  if (caseCountDiff > 0) {
    testCaseDiff = `:arrow_down_small: ${caseCountDiff}`;
  } else if (caseCountDiff < 0) {
    testCaseDiff = `:arrow_up_small: ${-caseCountDiff}`;
  }

  const shortBaseNextJsVersion = baseResults.nextjsVersion.split(" ")[1];
  const shortCurrentNextJsVersion =
    failedJobResults.nextjsVersion.split(" ")[1];
  // Append summary test report to the comment body
  let ret = `### Test summary
|   | ${
    shouldDiffWithMain
      ? `main (${baseResults.ref} / ${shortBaseNextJsVersion})`
      : `release (${baseResults.ref} / ${shortBaseNextJsVersion})`
  } | Current (${sha} / ${shortCurrentNextJsVersion}) | Diff |
|---|---|---|---|
| Failed Suites | ${baseTestFailedSuiteCount} | ${currentTestFailedSuiteCount} | ${testSuiteDiff} |
| Failed Cases | ${baseTestFailedCaseCount} | ${currentTestFailedCaseCount} | ${testCaseDiff} |

`;

  const fixedTests = baseTestFailedNames.filter(
    (name) => !currentTestFailedNames.includes(name)
  );
  const newFailedTests = currentTestFailedNames.filter(
    (name) => !baseTestFailedNames.includes(name)
  );

  if (fixedTests.length > 0) {
    ret += `\n:white_check_mark: **Fixed tests:**\n${fixedTests
      .map((t) => `\t- ${t}`)
      .join(" \n")}`;
  }

  if (newFailedTests.length > 0) {
    ret += `\n:x: **Newly failed tests:**\n${newFailedTests
      .map((t) => `\t- ${t}`)
      .join(" \n")}`;
  }

  // Store plain textbased summary to share into Slack channel
  // Note: Likely we'll need to polish this summary to make it more readable.
  if (shouldShareTestSummaryToSlack) {
    let textSummary = `*Next.js integration test status with Turbopack*

    *Base: ${baseResults.ref} / ${shortBaseNextJsVersion}*
    Failed suites: ${baseTestFailedSuiteCount}
    Failed cases: ${baseTestFailedCaseCount}

    *Current: ${sha} / ${shortCurrentNextJsVersion}*
    Failed suites: ${currentTestFailedSuiteCount}
    Failed cases: ${currentTestFailedCaseCount}

    `;

    if (suiteCountDiff === 0) {
      textSummary += "No changes in suite count.";
    } else if (suiteCountDiff > 0) {
      textSummary += `↓ ${suiteCountDiff} suites are fixed`;
    } else if (suiteCountDiff < 0) {
      textSummary += `↑ ${suiteCountDiff} suites are newly failed`;
    }

    if (caseCountDiff === 0) {
      textSummary += "No changes in test cases count.";
    } else if (caseCountDiff > 0) {
      textSummary += `↓ ${caseCountDiff} test cases are fixed`;
    } else if (caseCountDiff < 0) {
      textSummary += `↑ ${caseCountDiff} test cases are newly failed`;
    }

    console.log(
      "Storing text summary to ./test-summary.md to report into Slack channel.",
      textSummary
    );
    fs.writeFileSync("./test-summary.md", textSummary);
  }

  return ret;
}

// An action report failed next.js integration test with --turbo
async function run() {
  const { token, octokit, shouldDiffWithMain, prNumber, sha, existingComment } =
    await getInputs();

  // determine if we want to report summary into slack channel.
  // As a first step, we'll only report summary when the test is run against release-to-release. (no main branch regressions yet)
  const shouldReportSlack = !prNumber && !shouldDiffWithMain;

  // Collect current PR's failed test results
  const failedJobResults = await getFailedJobResults(octokit, token, sha);

  // Get the base to compare against
  const baseResults = await getTestResultDiffBase(octokit, shouldDiffWithMain);

  let fullCommentBody = "";
  if (failedJobResults.result.length === 0) {
    console.log("No failed test results found :tada:");
    fullCommentBody =
      `### Next.js test passes :green_circle: ${BOT_COMMENT_MARKER}` +
      `\nCommit: ${sha}\n`;
    return;
  } else {
    // Comment body to post test report with summary & full details.
    fullCommentBody =
      // Put the header title with marer comment to identify the comment for subsequent runs.
      `${commentTitlePre} ${BOT_COMMENT_MARKER}` + `\nCommit: ${sha}\n`;

    fullCommentBody += getTestSummary(
      sha,
      shouldDiffWithMain,
      baseResults,
      failedJobResults,
      shouldReportSlack
    );

    // Append full test report to the comment body, with collapsed <details>
    fullCommentBody += `\n<details>\n<summary>Full test report</summary>\n`;
    // Iterate over job results to construct full test report
    failedJobResults.result.forEach(
      ({ job, name: failedTest, data: testData }) => {
        // each job have nested array of test results
        // Fill in each individual test suite failures
        const groupedFails = {};
        const testResult = testData.testResults?.[0];
        const resultMessage = stripAnsi(testResult?.message);
        const failedAssertions = testResult?.assertionResults?.filter(
          (res) => res.status === "failed"
        );

        for (const fail of failedAssertions ?? []) {
          const ancestorKey = fail?.ancestorTitles?.join(" > ")!;

          if (!groupedFails[ancestorKey]) {
            groupedFails[ancestorKey] = [];
          }
          groupedFails[ancestorKey].push(fail);
        }

        if (existingComment?.body?.includes(sha)) {
          if (failedTest && existingComment.body?.includes(failedTest)) {
            console.log(
              `Suite is already included in current comment on ${prNumber}`
            );
            // the check_suite comment already says this test failed
            return;
          }
          fullCommentBody = existingComment.body;
        }

        fullCommentBody += `\n\`${failedTest}\` `;

        for (const group of Object.keys(groupedFails).sort()) {
          const fails = groupedFails[group];
          fullCommentBody +=
            `\n- ` +
            fails.map((fail) => `${group} > ${fail.title}`).join("\n- ");
        }

        fullCommentBody += `\n\n<details>`;
        fullCommentBody += `\n<summary>Expand output</summary>`;
        fullCommentBody += `\n\n${resultMessage}`;
        fullCommentBody += `\n</details>\n`;
      }
    );

    // Close </details>
    fullCommentBody += `</details>\n`;
  }

  try {
    if (!prNumber) {
      return;
    }

    if (!existingComment) {
      console.log("No existing comment found, creating a new one");
      const result = await octokit.rest.issues.createComment({
        ...context.repo,
        issue_number: prNumber,
        body: fullCommentBody,
      });

      console.log("Created a new comment", result.data.html_url);
    } else {
      console.log("Existing comment found, updating it");

      const result = await octokit.rest.issues.updateComment({
        ...context.repo,
        comment_id: existingComment.id,
        body: fullCommentBody,
      });

      console.log("Updated existing comment", result.data.html_url);
    }
  } catch (error) {
    console.error("Failed to post comment", error);

    // Comment update should succeed, otherwise let CI fails
    throw error;
  }
}

run();
