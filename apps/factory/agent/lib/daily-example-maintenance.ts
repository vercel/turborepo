export const DAILY_EXAMPLE_MAINTENANCE_PROMPT = `Call \`select_daily_example\` first. Maintain only the single example it returns; do not inspect or update any other example. Audit and update that example's stale dependencies, package-manager pin, Node engine, README instructions, versioned references, and \`turbo.json\` tasks. Use exact latest stable versions, apply required best-practice migrations, regenerate its lockfile with its declared package manager, and pass every relevant non-persistent validation task to one \`run_example_turbo_tasks\` call. Fix validation failures rather than stopping at an audit report.

Upgrade to whatever the registry publishes as latest without using \`minimumReleaseAgeExclude\` or any other release-age exclusion list, and do not add or change \`minimumReleaseAge\` configuration to make an install succeed.

When changes exist, call \`create_pull_request\` to open a draft pull request against \`vercel/turborepo\`. Summarize the selected example's changes and validation results in the body. The tool supplies the example-specific branch, Conventional Commit title, and commit message. If there are no changes, finish without creating a pull request.`;

export function fxExampleMaintenancePrompt(
  example: string,
  sessionId: string
): string {
  return `Maintain only the examples/${example} example; do not inspect or update any other example. Audit and update its stale dependencies, package-manager pin, Node engine, README instructions, versioned references, and turbo.json tasks. Use exact latest stable versions, apply required best-practice migrations, regenerate its lockfile with its declared package manager, and run every relevant non-persistent validation task. Fix validation failures rather than stopping at an audit report.

When changes exist, create a branch named agents/examples-${example}-fx-${sessionId}, commit with the title "chore: Update ${example} example", run \`gh auth setup-git\`, push it, and open a draft pull request against vercel/turborepo. Reuse an existing open pull request for that branch rather than creating a duplicate. Summarize the changes and validation results in the body. GH_TOKEN contains a non-secret placeholder; the sandbox firewall supplies GitHub authorization. If there are no changes, finish without creating a pull request.`;
}
