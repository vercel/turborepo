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
  revalidationKey: () => "turborepo-main-v1",
  async bootstrap({ use }) {
    const sandbox = await use();
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
    if (process.env.VERCEL) {
      await sandbox.setNetworkPolicy({
        allow: ["*"],
        subnets: {
          deny: [
            "0.0.0.0/8",
            "10.0.0.0/8",
            "100.64.0.0/10",
            "127.0.0.0/8",
            "169.254.0.0/16",
            "172.16.0.0/12",
            "192.168.0.0/16",
            "::1/128",
            "fc00::/7",
            "fe80::/10"
          ]
        }
      });
    }
  }
});
