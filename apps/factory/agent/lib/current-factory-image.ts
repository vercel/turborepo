import type { Sandbox } from "@vercel/sandbox";

import type { FactoryImagePointer } from "./factory-image-types";

export function requireFactoryImage(
  pointer: FactoryImagePointer | null
): FactoryImagePointer {
  if (pointer === null) {
    throw new Error(
      "No Factory image has been published. Build the shared image before creating chats."
    );
  }
  return pointer;
}

const CHECKOUT_REFRESH_TIMEOUT_MS = 2 * 60 * 1000;

export async function refreshFactoryCheckout(
  sandbox: Pick<Sandbox, "runCommand">,
  checkoutPath: string
): Promise<void> {
  const command = await sandbox.runCommand({
    args: [
      "-lc",
      "git fetch --depth=1 --force origin main && git reset --hard FETCH_HEAD"
    ],
    cmd: "bash",
    cwd: checkoutPath,
    timeoutMs: CHECKOUT_REFRESH_TIMEOUT_MS
  });
  if (command.exitCode !== 0) {
    const stderr = (await command.stderr()).trim();
    throw new Error(
      stderr
        ? `Could not update the workspace checkout to main: ${stderr}`
        : "Could not update the workspace checkout to main."
    );
  }
}
