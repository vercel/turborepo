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
  const command = ["sandbox", ...factorySandboxArgs(snapshotId)]
    .map((argument) =>
      /^[a-zA-Z0-9_./-]+$/.test(argument)
        ? argument
        : `'${argument.replaceAll("'", "'\\''")}'`
    )
    .join(" ");

  // These are shell expressions, not credentials. Resolve them only on the
  // operator's machine, never while rendering the page or building the image.
  return [
    command,
    '--env "GH_TOKEN=$(gh auth token)"',
    '--env "AI_GATEWAY_API_KEY=${AI_GATEWAY_API_KEY:?Set AI_GATEWAY_API_KEY locally first}"'
  ].join(" ");
}
