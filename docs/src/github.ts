import { setFailed } from "@actions/core";
import { context, getOctokit } from "@actions/github";
import { PullRequest } from "@octokit/webhooks-definitions/schema";
import { ReportRow } from "./config";

interface Comment {
  id: number;
}

export const COMMENT_TAG = "<!-- LINK_CHECKER_COMMENT -->";

type Octokit = ReturnType<typeof getOctokit>;

export const findBotComment = async (
  octokit: Octokit,
  pullRequest: PullRequest
): Promise<Comment | undefined> => {
  try {
    const { owner, repo } = context.repo;
    const { data: comments } = await octokit.rest.issues.listComments({
      owner,
      repo,
      issue_number: pullRequest.number,
    });

    return comments.find((c) => c.body?.includes(COMMENT_TAG));
  } catch (error) {
    setFailed("Error finding bot comment: " + error);
    return undefined;
  }
};

export const updateComment = async (
  octokit: Octokit,
  comment: string,
  botComment: Comment
): Promise<string> => {
  try {
    const { owner, repo } = context.repo;
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
};

export const createComment = async (
  octokit: Octokit,
  comment: string,
  pullRequest: PullRequest
): Promise<string> => {
  if (pullRequest.head.repo.fork) {
    setFailed(
      "The action could not create a GitHub comment because it is initiated from a forked repo. View the action logs for a list of broken links."
    );

    return "";
  } else {
    try {
      const { owner, repo } = context.repo;
      const { data } = await octokit.rest.issues.createComment({
        owner,
        repo,
        issue_number: pullRequest.number,
        body: comment,
      });

      return data.html_url;
    } catch (error) {
      setFailed("Error creating comment: " + error);
      return "";
    }
  }
};

export const updateCheckStatus = async (
  octokit: Octokit,
  errorsExist: boolean,
  commentUrl: string | undefined,
  pullRequest: PullRequest
): Promise<void> => {
  let summary, text;

  if (errorsExist) {
    summary =
      "This PR introduces broken links to the docs. Click details for a list.";
    text = `[See the comment for details](${commentUrl})`;
  } else {
    summary = "No broken links found";
  }

  const { owner, repo } = context.repo;
  const title = "Docs Link Validation";
  const checkParams = {
    owner,
    repo,
    name: title,
    head_sha: pullRequest.head.sha,
    status: "completed",
    conclusion: errorsExist ? "failure" : "success",
    output: {
      title,
      summary,
      text,
    },
  } as const;

  if (pullRequest.head.repo.fork) {
    if (errorsExist) {
      setFailed(
        "This PR introduces broken links to the docs. The action could not create a GitHub check because it is initiated from a forked repo."
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
};

export const reportErrorsToGitHub = async (reportRows: ReportRow[]) => {
  const octokit = getOctokit(process.env.GITHUB_TOKEN!);

  const pullRequest = context.payload.pull_request as PullRequest;

  if (!pullRequest) {
    return;
  }

  try {
    const botComment = await findBotComment(octokit, pullRequest);
    let commentUrl: string;

    if (reportRows.length > 0) {
      const errorComment = [
        "Hi there :wave:",
        "",
        "",
        "It looks like this PR introduces broken links to the docs, please take a moment to fix them before merging:",
        "",
        "",
        "| Broken link | Type | File |",
        "| ----------- | ----------- | ----------- |",
        ...reportRows,
        "",
        "Thank you :pray:",
      ].join("\n");

      let comment;

      comment = `${COMMENT_TAG}\n${errorComment}`;
      if (botComment) {
        commentUrl = await updateComment(octokit, comment, botComment);
      } else {
        commentUrl = await createComment(octokit, comment, pullRequest);
      }
      process.exit(1);
    }

    if (botComment) {
      const comment = `${COMMENT_TAG}\nAll broken links are now fixed, thank you!`;
      commentUrl = await updateComment(octokit, comment, botComment);
    } else {
      commentUrl = ""; // ??
    }

    try {
      await updateCheckStatus(
        octokit,
        reportRows.length > 0,
        commentUrl,
        pullRequest
      );
    } catch (error) {
      setFailed("Failed to create GitHub check: " + error);
    }
  } catch (error) {
    setFailed("Error validating internal links: " + error);
  }
};
