import { ChildProcess, spawn } from "child_process";
import split2 from "split2";
import treeKill from "tree-kill";

export interface Command {
  ok(): Promise<void>;
  kill(): Promise<void>;
  end(): Promise<number>;
  waitForOutput(regex: RegExp): Promise<RegExpMatchArray>;
  stdout: string;
  stderr: string;
  output: string;
}

const shellOutput = !!process.env.SHELL_OUTPUT;

class CommandImpl {
  stdout: string = "";
  stderr: string = "";
  output: string = "";
  exitPromise: Promise<number>;
  waitingForOutput: (() => void)[] = [];
  constructor(private process: ChildProcess) {
    process.stdout?.pipe(split2()).on("data", (data) => {
      const str = data.toString();
      this.stdout += str + "\n";
      this.output += str + "\n";
      if (shellOutput) {
        console.log(`[STDOUT] ${str}`);
      }
      if (this.waitingForOutput.length !== 0) {
        const waitingForOutput = this.waitingForOutput;
        this.waitingForOutput = [];
        for (const fn of waitingForOutput) {
          fn();
        }
      }
    });
    process.stderr?.pipe(split2()).on("data", (data) => {
      const str = data.toString();
      this.stderr += str + "\n";
      this.output += str + "\n";
      if (shellOutput) {
        console.log(`[STDERR] ${str}`);
      }
      if (this.waitingForOutput.length !== 0) {
        const waitingForOutput = this.waitingForOutput;
        this.waitingForOutput = [];
        for (const fn of waitingForOutput) {
          fn();
        }
      }
    });
    this.exitPromise = new Promise<number>((resolve, reject) => {
      process.on("error", reject);
      process.on("exit", resolve);
    });
  }

  async ok() {
    const exitCode = await this.exitPromise;
    if (exitCode !== 0) {
      throw new Error(
        `Command exited with code ${exitCode}\n\nOutput:\n${this.output}`
      );
    }
  }

  async end() {
    return await this.exitPromise;
  }

  async kill() {
    const pid = this.process.pid!;
    await new Promise<void>((resolve, reject) =>
      treeKill(pid, (err) => {
        if (err) reject(err);
        else resolve();
      })
    );
    await this.exitPromise;
  }

  async waitForOutput(regex: RegExp) {
    let start = this.output.length;
    while (true) {
      const match = this.output.slice(start).match(regex);
      if (match) {
        return match;
      }
      const waitResult = await Promise.race([
        this.exitPromise,
        new Promise<void>((resolve) => {
          this.waitingForOutput.push(resolve);
        }).then(() => "output"),
      ]);
      if (waitResult !== "output") {
        throw new Error(
          `Command exited with code ${waitResult}\n\nOutput:\n${this.output}`
        );
      }
    }
  }
}

export function command(
  command: string,
  args: string[],
  options: {
    env?: Record<string, string>;
    cwd?: string;
  } = {}
): Command {
  const process = spawn(command, args, {
    ...options,
    stdio: ["ignore", "pipe", "pipe"],
  });
  if (shellOutput) {
    console.log(
      `[SHELL] ${command} ${args.join(" ")} ${JSON.stringify(options)}`
    );
  }
  return new CommandImpl(process);
}
