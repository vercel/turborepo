import * as core from "@actions/core";
import { context, getOctokit } from "@actions/github";

type Octokit = ReturnType<typeof getOctokit>;

type Tag = {
  name: string;
  sha: string;
  date: number;
  patch: number;
};

/**
 * inputs:
 *   - github_token
 *   - commit_sha
 *   - prefix
 * outputs:
 *   - new_tag
 */
async function run() {
  const githubToken = core.getInput("github_token", { required: true });
  const prefix = core.getInput("prefix");
  const commitSha = core.getInput("commit_sha") || process.env.GITHUB_SHA!;

  const octokit = getOctokit(githubToken);

  const tagFilter = new RegExp(
    String.raw`^${prefix}(?<date>\d{6})\.(?<patch>\d+)$`
  );
  const tags = await getTags(octokit, tagFilter);

  core.info("found tags:");
  core.info(JSON.stringify(tags, null, 2));

  const lastTag =
    tags.pop() ||
    ({
      // Just any date that's not today.
      name: `${prefix}230101.0`,
      sha: "HEAD",
      date: 230101,
      patch: 0,
    } satisfies Tag);

  const today = getToday();
  const nextPatch = today === lastTag.date ? lastTag.patch + 1 : 1;
  const nextTag = `${prefix}${today}.${nextPatch}`;

  core.info(JSON.stringify({ today, lastTag, nextTag }));

  core.setOutput("new_tag", nextTag);
  await createTag(octokit, nextTag, commitSha);
  core.notice(`New tag is ${nextTag}`);

  // TODO: generate real release notes
  core.setOutput(
    "changelog",
    `See the commit diff at https://github.com/vercel/turbo/compare/${lastTag.name}...${nextTag}`
  );
}

/**
 * Returns the current date in YYMMDD
 */
function getToday() {
  const now = new Date();
  const year = now.getFullYear() % 100;
  // Did you know JS's getMonth is 0 based, but getDate is 1 based? Java man.
  const month = now.getMonth() + 1;
  const day = now.getDate();
  return year * 10000 + month * 100 + day * 1;
}

/**
 * Gets the latest tags that pass the filter regex.
 */
async function getTags(
  octokit: Octokit,
  filter: RegExp,
  page = 0
): Promise<Tag[]> {
  const resp = await octokit.rest.repos.listTags({
    ...context.repo,
    per_page: 100,
    page,
  });

  if (resp.data.length === 0) return [];

  const tags = resp.data.filter((tag) => filter.test(tag.name));

  // If we had tags on this page, but none passed the filter, then continue on
  // to the next page.
  if (tags.length === 0) return getTags(octokit, filter, page + 1);

  return tags
    .map((tag) => {
      const match = filter.exec(tag.name)!;
      return {
        name: tag.name,
        sha: tag.commit.sha,
        date: Number(match.groups!.date),
        patch: Number(match.groups!.patch),
      };
    })
    .sort((a, b) => {
      // Sort ascending, first by the date, then by patch number.
      const date = a.date - b.date;
      if (date !== 0) return date;
      const patch = a.patch - b.date;
      return patch;
    });
}

/**
 * Creates a light tag by creating a new reference in `refs/tags/`
 */
async function createTag(octokit: Octokit, newTag: string, sha: string) {
  await octokit.rest.git.createRef({
    ...context.repo,
    ref: `refs/tags/${newTag}`,
    sha,
  });
}

run().catch(core.setFailed);
