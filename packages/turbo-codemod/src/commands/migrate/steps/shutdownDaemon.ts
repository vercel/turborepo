import type { ExecSyncOptions } from "node:child_process";
import type { Project } from "@turbo/workspaces";
import { exec } from "../utils";

export function shutdownDaemon({ project }: { project: Project }) {
  try {
    const execOpts: ExecSyncOptions = {
      cwd: project.paths.root,
      stdio: "ignore",
    };
    // see if we have a global install
    const turboBinaryPathFromGlobal = exec(`turbo bin`, execOpts);
    // if we do, shut it down
    if (turboBinaryPathFromGlobal) {
      exec(`turbo daemon stop`, execOpts);
    } else {
      // call turbo using the project package manager to shut down the daemon
      let command = `${project.packageManager} turbo daemon stop`;
      if (project.packageManager === "npm") {
        command = `npm exec -c 'turbo daemon stop'`;
      }

      exec(command, execOpts);
    }
  } catch (e) {
    // skip
  }
}
