import { Octokit } from "@octokit/rest";
import crypto from "node:crypto";
import { githubToken, githubWebhookSecret } from "./env";

const OWNER = "vercel";
const REPO = "turborepo";

let _octokit: Octokit | undefined;
export function octokit(): Octokit {
  if (!_octokit) {
    _octokit = new Octokit({ auth: githubToken() });
  }
  return _octokit;
}

export async function verifyGitHubWebhook(
  request: Request,
  rawBody: string
): Promise<void> {
  const signature = request.headers.get("x-hub-signature-256");
  if (!signature) {
    throw new Error("Missing GitHub webhook signature");
  }

  const hmac = crypto.createHmac("sha256", githubWebhookSecret());
  hmac.update(rawBody);
  const expected = `sha256=${hmac.digest("hex")}`;

  if (!crypto.timingSafeEqual(Buffer.from(signature), Buffer.from(expected))) {
    throw new Error("Invalid GitHub webhook signature");
  }
}

export async function getIssue(issueNumber: number) {
  const { data } = await octokit().issues.get({
    owner: OWNER,
    repo: REPO,
    issue_number: issueNumber
  });
  return data;
}

export async function addComment(issueNumber: number, body: string) {
  await octokit().issues.createComment({
    owner: OWNER,
    repo: REPO,
    issue_number: issueNumber,
    body
  });
}

export async function addLabels(issueNumber: number, labels: string[]) {
  await octokit().issues.addLabels({
    owner: OWNER,
    repo: REPO,
    issue_number: issueNumber,
    labels
  });
}

export async function createPullRequest(opts: {
  title: string;
  body: string;
  head: string;
  base?: string;
}) {
  const { data } = await octokit().pulls.create({
    owner: OWNER,
    repo: REPO,
    title: opts.title,
    body: opts.body,
    head: opts.head,
    base: opts.base ?? "main"
  });
  return data;
}
