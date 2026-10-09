/** Create a fresh sandbox from the published image and attach an interactive shell. */
export function factorySandboxArgs(snapshotId: string): string[] {
  if (!snapshotId.trim()) {
    throw new Error("No factory image has been published yet.");
  }
  return [
    "create",
    "--snapshot",
    snapshotId,
    "--vcpus",
    "8",
    "--timeout",
    "45m",
    "--connect"
  ];
}

export function factorySandboxCommand(snapshotId: string): string {
  return ["sandbox", ...factorySandboxArgs(snapshotId)]
    .map((argument) =>
      /^[a-zA-Z0-9_./-]+$/.test(argument)
        ? argument
        : `'${argument.replaceAll("'", "'\\''")}'`
    )
    .join(" ");
}
