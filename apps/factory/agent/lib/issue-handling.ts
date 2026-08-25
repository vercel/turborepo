import type { SandboxSession } from "eve/sandbox";

export const FACTORY_ISSUE_ATTRIBUTE = "factory_automatic_issue";

const stateDirectory = ".issue-handling";

export type IssueConfidence = "low" | "medium" | "high";

export interface IssueAssessment {
  readonly confidence: IssueConfidence | null;
  readonly confidenceReason: string | null;
  readonly issueNumber: number;
  readonly issueTitle: string;
  readonly issueUrl: string;
  readonly safe: boolean;
  readonly securityReason: string;
}

type Auth =
  | {
      readonly authenticator?: string;
      readonly attributes?: Readonly<
        Record<string, string | readonly string[]>
      >;
    }
  | null
  | undefined;

export function isAutomaticIssueSession(auth: Auth): boolean {
  return (
    auth?.authenticator === "github-webhook" &&
    auth.attributes?.[FACTORY_ISSUE_ATTRIBUTE] === "true"
  );
}

export function validateIssueAssessment(
  assessment: IssueAssessment
): IssueAssessment {
  if (!Number.isSafeInteger(assessment.issueNumber) || assessment.issueNumber <= 0) {
    throw new Error("Issue number must be a positive integer.");
  }
  const expectedUrl = `https://github.com/vercel/turborepo/issues/${assessment.issueNumber}`;
  if (assessment.issueUrl !== expectedUrl) {
    throw new Error("Issue URL does not match vercel/turborepo.");
  }
  if (!assessment.issueTitle.trim() || !assessment.securityReason.trim()) {
    throw new Error("Issue title and security reason are required.");
  }
  if (assessment.safe) {
    if (assessment.confidence === null || !assessment.confidenceReason?.trim()) {
      throw new Error("Safe issues require a confidence assessment and reason.");
    }
  } else if (
    assessment.confidence !== null ||
    assessment.confidenceReason !== null
  ) {
    throw new Error("Blocked issues cannot include a confidence assessment.");
  }
  return assessment;
}

export async function recordIssueAssessment(
  sandbox: SandboxSession,
  sessionId: string,
  assessment: IssueAssessment
): Promise<IssueAssessment> {
  const validated = validateIssueAssessment(assessment);
  const directory = await sandbox.run({ command: `mkdir -p ${stateDirectory}` });
  if (directory.exitCode !== 0) throw new Error(directory.stderr);
  await sandbox.writeTextFile({
    path: statePath(sessionId),
    content: `${JSON.stringify(validated)}\n`
  });
  return validated;
}

export async function readIssueAssessment(
  sandbox: SandboxSession,
  sessionId: string
): Promise<IssueAssessment | null> {
  const content = await sandbox.readTextFile({ path: statePath(sessionId) });
  if (content === null) return null;
  return validateIssueAssessment(JSON.parse(content) as IssueAssessment);
}

export async function requireActionableIssueAssessment(
  sandbox: SandboxSession,
  sessionId: string
): Promise<IssueAssessment> {
  const assessment = await readIssueAssessment(sandbox, sessionId);
  if (!assessment?.safe) {
    throw new Error("Automatic issue handling has not passed security triage.");
  }
  if (assessment.confidence === "low") {
    throw new Error("Low-confidence issues must produce a report, not a pull request.");
  }
  return assessment;
}

function statePath(sessionId: string): string {
  return `${stateDirectory}/${encodeURIComponent(sessionId)}.json`;
}
