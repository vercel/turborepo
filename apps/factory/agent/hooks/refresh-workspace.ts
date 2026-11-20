import { defineHook } from "eve/hooks";

const WORKSPACE_CHECKOUT_PATH = "/factory/turborepo";

export default defineHook({
  events: {
    async "session.started"(_event, ctx) {
      const sandbox = await ctx.getSandbox();
      const result = await sandbox.run({
        command: `git -C ${WORKSPACE_CHECKOUT_PATH} fetch --depth=1 --force origin main && git -C ${WORKSPACE_CHECKOUT_PATH} reset --hard FETCH_HEAD`
      });
      if (result.exitCode !== 0) {
        throw new Error(
          result.stderr || "Could not update the workspace checkout to main."
        );
      }
    }
  }
});
