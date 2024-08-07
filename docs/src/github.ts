import * as github from "@actions/github";
import { setFailed } from "@actions/core";

interface Comment {
  id: number;
}

export { setFailed };
export const COMMENT_TAG = "<!-- LINK_CHECKER_COMMENT -->";

const { context, getOctokit } = github;
const octokit = getOctokit(process.env.GITHUB_TOKEN!);
const { owner, repo } = context.repo;
const pullRequest = context.payload.pull_request;
if (!pullRequest) {
  console.log("Skipping since this is not a pull request");
  process.exit(0);
}
export const sha = pullRequest.head.sha;
const isFork = pullRequest.head.repo.fork;
const prNumber = pullRequest.number;

export async function findBotComment(): Promise<Comment | undefined> {
  try {
    const { data: comments } = await octokit.rest.issues.listComments({
      owner,
      repo,
      issue_number: prNumber,
    });

    return comments.find((c) => c.body?.includes(COMMENT_TAG));
  } catch (error) {
    setFailed("Error finding bot comment: " + error);
    return undefined;
  }
}

export async function updateComment(
  comment: string,
  botComment: Comment
): Promise<string> {
  try {
    const { data } = await octokit.rest.issues.updateComment({
      owner,
      repo,
      comment_id: botComment.id,
      body: comment,
    });

    return data.html_url;
  } catch (error) {
    setFailed("Error updating comment: " + error);
    return "";
  }
}

export async function createComment(comment: string): Promise<string> {
  if (isFork) {
    setFailed(
      "The action could not create a Github comment because it is initiated from a forked repo. View the action logs for a list of broken links."
    );

    return "";
  } else {
    try {
      const { data } = await octokit.rest.issues.createComment({
        owner,
        repo,
        issue_number: prNumber,
        body: comment,
      });

      return data.html_url;
    } catch (error) {
      setFailed("Error creating comment: " + error);
      return "";
    }
  }
}

export async function updateCheckStatus(
  errorsExist: boolean,
  commentUrl?: string
): Promise<void> {
  const checkName = "Docs Link Validation";

  let summary, text;

  if (errorsExist) {
    summary =
      "This PR introduces broken links to the docs. Click details for a list.";
    text = `[See the comment for details](${commentUrl})`;
  } else {
    summary = "No broken links found";
  }

  const checkParams = {
    owner,
    repo,
    name: checkName,
    head_sha: sha,
    status: "completed",
    conclusion: errorsExist ? "failure" : "success",
    output: {
      title: checkName,
      summary: summary,
      text: text,
    },
  };

  if (isFork) {
    if (errorsExist) {
      setFailed(
        "This PR introduces broken links to the docs. The action could not create a Github check because it is initiated from a forked repo."
      );
    } else {
      console.log("Link validation was successful.");
    }
  } else {
    try {
      await octokit.rest.checks.create(checkParams);
    } catch (error) {
      setFailed("Failed to create check: " + error);
    }
  }
}
