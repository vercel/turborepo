const { context, getOctokit } = require('@actions/github');
const { info, getInput } = require('@actions/core');

async function run() {
  const token = getInput('token');
  const octokit = getOctokit(token);

  const prNumber = context?.payload?.pull_request?.number;

  console.log('job', { id: context?.job, runId: context?.runId });
  console.log('PR Number: ', prNumber);
  if (!prNumber) {
    info('No PR number found in context, exiting');
    return;
  }


  ///listJobsForWorkflowRun
  //listWorkflowRuns
  const pr = await octokit.rest.pulls.get({
    ...context.repo,
    pull_number: prNumber,
  });

  const jobs = await octokit.rest.actions.listJobsForWorkflowRun({
    ...context.repo,
    run_id: context?.runId
  });

  const integrationTestJob = jobs?.data?.jobs?.find(job => job?.name?.includes('next_js_integration'));

  console.log('========================================================= job');
  console.log(integrationTestJob);
  console.log('========================================================= job');


  const jobLogs = await octokit.rest.actions.downloadWorkflowRunLogs({
    ...context.repo,
    run_id: integrationTestJob?.run_id
  });

  console.log('========================================================= joblogs');
  console.log(jobLogs);
  console.log('========================================================= joblogs');

  const comments = await octokit.rest.issues.listComments({
    ...context.repo,
    issue_number: prNumber,
  });

  const existingComment = comments?.data.find(comment => comment?.user?.login === 'github-actions[bot]' && comment?.body?.includes('<!-- __marker__ next.js integration stats __marker__ -->'));

  if (!existingComment) {
    info('No existing comment found, creating a new one');
    await octokit.rest.issues.createComment({
      ...context.repo,
      issue_number: prNumber,
      body: `
      test comment ${Date.now()}
      <!-- __marker__ next.js integration stats __marker__ -->
      `,
    });
    return;
  }
}

run();