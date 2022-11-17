import pkg from "../package.json";
import { TurboIgnoreArgs } from "./types";

export const help = `
turbo-ignore

Automatically ignore builds that have no changes

Usage:
  $ npx turbo-ignore [<workspace>] [flags...]

If <workspace> is not provided, it will be inferred from the "name"
field of the "package.json" located at the current working directory.

Flags:
  --fallback=<ref>    On Vercel, if no previously deployed SHA is available to compare against,
                      fallback to comparing against the provided ref, or use "false" to
                      disable [default: HEAD^]
  --help, -h          Show this help message
  --version, -v       Show the version of this script
`;

// simple args parser because we don't want to pull in a dependency
// and we don't need many features
export default function parseArgs({
  argv,
}: {
  argv: Array<string>;
}): TurboIgnoreArgs {
  const args: TurboIgnoreArgs = { directory: process.cwd() };

  // find all flags
  const flags = new Set(
    argv
      .filter((args) => args.startsWith("-"))
      .map((flag) => flag.replace(/-/g, ""))
  );

  // handle help flag and exit
  if (flags.has("help") || flags.has("h")) {
    console.log(help);
    process.exit(0);
  }
  // handle version flag and exit
  if (flags.has("version") || flags.has("v")) {
    console.log(pkg.version);
    process.exit(0);
  }

  // set workspace (if provided)
  if (argv.length && !argv[0].startsWith("-")) {
    args.workspace = argv[0];
  }

  // set fallback (if provided)
  const fallbackArg = argv.find((arg) => arg.startsWith("--fallback="));
  if (fallbackArg) {
    args.fallback = fallbackArg.split("=")[1];
  }

  return args;
}
