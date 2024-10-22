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
  let summary, text, conclusion;

  if (errorsExist) {
    summary =
      "This PR introduces broken links to the docs. Click details for a list.";
    text = `[See the comment for details](${commentUrl})`;
    conclusion = "failure" as const;

    if (pullRequest.head.repo.fork) {
      setFailed(
        "This PR introduces broken links to the docs. The action could not create a GitHub check because it is initiated from a forked repo."
      );
    }
  } else {
    summary = "No broken links found";
    conclusion = "success" as const;
    text = "";
  }

  const title = "Docs Link Validation";
  const checkParams = {
    owner: context.repo.owner,
    repo: context.repo.repo,
    name: title,
    head_sha: pullRequest.head.sha,
    status: "completed",
    conclusion,
    output: {
      title,
      summary,
      text,
    },
  } as const;

  if (!pullRequest.head.repo.fork) {
    try {
      await octokit.rest.checks.create(checkParams);
    } catch (error) {
      setFailed("Failed to create check: " + error);
    }
  }
};

export const reportErrorsToGitHub = async (reportRows: ReportRow[]) => {
  const { GITHUB_TOKEN, GITHUB_REPOSITORY, CI, GITHUB_ACTIONS } = process.env;

  if (!CI) {
    // we only want to run this in CI
    return;
  }

  if (!GITHUB_ACTIONS) {
    // we only want to run this in GitHub Actions
    return;
  }

  if (!GITHUB_TOKEN) {
    throw new Error("No GITHUB_TOKEN found, skipping GitHub reporting");
  }

  if (!GITHUB_REPOSITORY) {
    throw new Error("No GITHUB_REPOSITORY found, skipping GitHub reporting");
  }

  const octokit = getOctokit(GITHUB_TOKEN);

  const pullRequest = context.payload.pull_request as PullRequest;

  if (!pullRequest) {
    throw new Error("No pullRequest found, skipping GitHub reporting");
  }

  try {
    const botComment = await findBotComment(octokit, pullRequest);

    if (reportRows.length > 0) {
      const errorComment = [
        "Hi there :wave:",
        "",
        "",
        "It looks like this PR introduces broken links to the docs, please take a moment to fix them before merging:",
        "",
        "",
        "| Broken link | Type | File |",
        "| ----------- | ---- | ---- |",
        ...reportRows.map(({ link, path, type }) => {
          const docPath = path.replace("../../../", "");
          return `| ${link} | ${type} | [/${docPath}](https://github.com/vercel/turborepo/blob/${pullRequest.head.sha}/${docPath}) |`;
        }),
        "",
        "Thank you :pray:",
      ].join("\n");

      await updateComment(
        octokit,
        `${COMMENT_TAG}\n${errorComment}`,
        botComment ?? pullRequest
      );
      process.exit(1);
    }

    let commentUrl: string;
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
