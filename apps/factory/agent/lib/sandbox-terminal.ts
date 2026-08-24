import { Sandbox } from "@vercel/sandbox";

import { FACTORY_IMAGE_SPEC } from "#factory-image";

export interface TerminalSession {
  readonly token: string;
  readonly url: string;
}

interface CommandResult {
  readonly exitCode: number;
}

interface CommandRunner {
  runCommand(
    command: string,
    args?: string[],
    options?: { readonly timeoutMs?: number }
  ): Promise<CommandResult>;
}

interface InteractiveSandbox extends CommandRunner {
  asUser(name: "root"): CommandRunner;
  openInteractive(): Promise<TerminalSession>;
}

export async function createTerminalSession(
  sandboxName: string,
  getSandbox: (name: string) => Promise<InteractiveSandbox> = async (name) =>
    Sandbox.get({ name, resume: true })
): Promise<TerminalSession> {
  const sandbox = await getSandbox(sandboxName);
  await ensureFxInstalled(sandbox);
  return sandbox.openInteractive();
}

export async function ensureFxInstalled(
  sandbox: InteractiveSandbox
): Promise<void> {
  const probe = await sandbox.runCommand("sh", ["-c", "command -v fx >/dev/null"]);
  if (probe.exitCode === 0) return;

  const version = FACTORY_IMAGE_SPEC.fxVersion;
  const script = String.raw`set -euo pipefail
case "$(uname -m)" in
  x86_64) arch=x86_64; checksum=${FACTORY_IMAGE_SPEC.fxSha256.x86_64} ;;
  aarch64|arm64) arch=aarch64; checksum=${FACTORY_IMAGE_SPEC.fxSha256.aarch64} ;;
  *) echo "Unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac
archive="/tmp/fx-linux-$arch.tar.gz"
curl --fail --show-error --silent --location --output "$archive" \
  "https://github.com/vercel-labs/fx/releases/download/v${version}/fx-linux-$arch.tar.gz"
printf '%s  %s\n' "$checksum" "$archive" | sha256sum --check --strict
tar -xzf "$archive" -C /tmp fx
install -m 0755 /tmp/fx /usr/local/bin/fx
rm -f /tmp/fx "$archive"
FX_AUTO_UPGRADE=0 fx --version`;
  const installed = await sandbox
    .asUser("root")
    .runCommand("bash", ["-lc", script], { timeoutMs: 60_000 });
  if (installed.exitCode !== 0)
    throw new Error("Could not install fx in this older sandbox.");
}
