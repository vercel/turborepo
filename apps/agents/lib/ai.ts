import { generateText } from "ai";

const MODEL = "anthropic/claude-sonnet-4-5";

export async function classifyIssueForReproduction(
  issueTitle: string,
  issueBody: string
): Promise<{ needsReproduction: boolean; reasoning: string }> {
  const { text } = await generateText({
    model: MODEL,
    system: `You analyze GitHub issues for the Turborepo repository to determine if the reporter provided a reproduction.

A reproduction means ANY of:
- A link to a GitHub repository demonstrating the issue
- A link to a StackBlitz, CodeSandbox, or similar playground
- Step-by-step instructions that include actual commands to run against a specific setup
- Inline code that fully demonstrates the problem (not just a snippet or error log)

The following are NOT reproductions:
- Just an error message or stack trace
- A description of the problem without steps to recreate it
- "It happens in my project" without a way to reproduce
- Screenshots alone
- Feature requests (these don't need reproductions)
- Questions (these don't need reproductions)

Respond with exactly one line: YES or NO, followed by a pipe character, followed by a brief reason.
YES means a reproduction IS needed (i.e. one was NOT provided).
NO means a reproduction is NOT needed (one was provided, or the issue is a feature request / question).

Examples:
YES|No reproduction link or steps provided, just an error message.
NO|Includes a link to a GitHub repo demonstrating the issue.
NO|This is a feature request, not a bug report.`,
    prompt: `Issue title: ${issueTitle}\n\nIssue body:\n${issueBody || "(empty)"}`
  });

  const [verdict, ...reasonParts] = text.trim().split("|");
  return {
    needsReproduction: verdict?.trim().toUpperCase() === "YES",
    reasoning: reasonParts.join("|").trim() || "No reasoning provided"
  };
}
