import { convert } from "@turbo/workspaces";
import { TransformInput, TransformResult } from "./types";

const meta = {
  name: "package-manager",
};

export async function transform(args: TransformInput): TransformResult {
  const { project, prompts } = args;
  const { root, packageManager } = prompts;

  if (packageManager && project.packageManager !== packageManager.name) {
    await convert({
      root,
      to: packageManager.name,
      options: {
        // skip install after conversion- we will do it later
        skipInstall: true,
      },
    });
  } else {
    return { result: "not-applicable", ...meta };
  }

  return { result: "success", ...meta };
}
