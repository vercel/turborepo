import { Sandbox } from "@vercel/sandbox";

export interface TerminalSession {
  readonly url: string;
  readonly token: string;
}

export interface SandboxWithInteractive {
  readonly openInteractive: (opts?: {
    readonly signal?: AbortSignal;
  }) => Promise<{ readonly url: string; readonly token: string }>;
}

const ALLOWED_SANDBOX_NAME_PREFIX = "ai-sdk-harness";

export function isAllowedSandboxName(name: string): boolean {
  return name.startsWith(ALLOWED_SANDBOX_NAME_PREFIX);
}

export async function createTerminalSession(
  sandboxName: string,
  getSandbox: (
    name: string
  ) => Promise<SandboxWithInteractive> = defaultGetSandbox
): Promise<TerminalSession> {
  if (!isAllowedSandboxName(sandboxName)) {
    throw new Error(
      `Sandbox name must start with "${ALLOWED_SANDBOX_NAME_PREFIX}".`
    );
  }

  const sandbox = await getSandbox(sandboxName);
  const session = await sandbox.openInteractive();
  return { url: session.url, token: session.token };
}

async function defaultGetSandbox(
  name: string
): Promise<SandboxWithInteractive> {
  const sandbox = await Sandbox.get({ name, resume: true });
  return sandbox as SandboxWithInteractive;
}

export async function handleTerminalRequest(
  request: Request,
  createSession: (
    name: string
  ) => Promise<TerminalSession> = createTerminalSession
): Promise<Response> {
  let body: unknown;
  try {
    body = await request.json();
  } catch {
    return Response.json({ error: "Invalid JSON body." }, { status: 400 });
  }

  if (!isValidRequest(body)) {
    return Response.json(
      { error: "A valid sandboxName is required." },
      { status: 400 }
    );
  }

  const { sandboxName } = body;

  if (!isAllowedSandboxName(sandboxName)) {
    return Response.json(
      { error: "Sandbox name is not allowed." },
      { status: 400 }
    );
  }

  try {
    const session = await createSession(sandboxName);
    return Response.json(session);
  } catch (error) {
    const message =
      error instanceof Error
        ? error.message
        : "Could not open an interactive session for the sandbox.";
    const status = message.includes("not_found") ? 404 : 500;
    return Response.json({ error: message }, { status });
  }
}

function isValidRequest(
  value: unknown
): value is { readonly sandboxName: string } {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as Record<string, unknown>).sandboxName === "string"
  );
}
