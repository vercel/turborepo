import { defineSandbox, type SandboxSession } from "eve/sandbox";

const repository = "https://github.com/vercel/turborepo.git";
const checkout = "turborepo";

async function runOrThrow(sandbox: SandboxSession, command: string) {
  const result = await sandbox.run({ command });
  if (result.exitCode !== 0) {
    throw new Error(`${command} failed: ${result.stderr}`);
  }
}

export default defineSandbox({
  revalidationKey: () => "turborepo-main-opencode-1.18.16-v1",
  async bootstrap({ use }) {
    const sandbox = await use();
    await runOrThrow(sandbox, "npm install --global opencode-ai@1.18.16");
    await runOrThrow(
      sandbox,
      `git clone --depth=1 --branch=main ${repository} ${checkout}`
    );
  },
  async onSession({ use }) {
    const sandbox = await use();
    await runOrThrow(
      sandbox,
      `git -C ${checkout} fetch --depth=1 origin main && git -C ${checkout} reset --hard FETCH_HEAD && git -C ${checkout} clean -ffd`
    );
  }
});
