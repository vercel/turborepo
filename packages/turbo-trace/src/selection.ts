import { appendFileSync, existsSync, readFileSync } from "node:fs";
import { dirname, join, relative, sep } from "node:path";

/** The selection of `turbo trace affected --for-runners`, as a plugin uses it. */
export interface Selection {
  /** The repository root, when the selection's files are found from here. */
  root?: string;
  /** The test files the trace knows, relative to the repository root. */
  known: string[];
  /** Those of them that the change cannot affect: the only files dropped. */
  dropped: Set<string>;
}

const everything: Selection = { known: [], dropped: new Set() };
let loaded: { path: string; selection: Selection } | undefined;

function warn(why: string): Selection {
  process.stderr.write(`turbo trace: ${why}: running every test\n`);
  return everything;
}

const strings = (value: unknown): value is string[] =>
  Array.isArray(value) && value.every((item) => typeof item === "string");

/**
 * The outermost directory around the working directory that holds `sample`,
 * a path of the selection. A wrong answer is a directory further out, and
 * then no path matches the selection: every test runs.
 */
function findRoot(sample: string): string | undefined {
  const dirs = [process.cwd()];
  while (dirname(dirs[0]) !== dirs[0]) {
    dirs.unshift(dirname(dirs[0]));
  }
  return dirs.find((dir) => existsSync(join(dir, sample)));
}

function read(path: string): Selection {
  let doc: Record<string, unknown>;
  try {
    doc = JSON.parse(readFileSync(path, "utf8")) ?? {};
  } catch (error) {
    return warn(`cannot read the selection ${path} (${error})`);
  }
  const { files, runWhole, unselected } = doc;
  if (
    doc.version !== 1 ||
    !strings(files) ||
    !strings(runWhole) ||
    !strings(unselected)
  ) {
    return warn(`the selection ${path} is not a version 1 document`);
  }
  const known = [...files, ...unselected];
  const root = known.length > 0 ? findRoot(known[0]) : undefined;
  if (doc.runAll !== false || unselected.length === 0) {
    return { root, known, dropped: new Set() };
  }
  if (root === undefined) {
    return warn(`the files of the selection ${path} are not around here`);
  }
  const whole = (file: string) => runWhole.some((dir) => file.startsWith(dir));
  const dropped = new Set(unselected.filter((file) => !whole(file)));
  return { root, known, dropped };
}

/**
 * The selection named by `TURBO_TRACE_SELECTION`. Without the variable there
 * is none. A document that cannot be used drops nothing, with one warning.
 */
export function loadSelection(): Selection | undefined {
  const path = process.env.TURBO_TRACE_SELECTION;
  if (!path) {
    return undefined;
  }
  const selection = loaded?.path === path ? loaded.selection : read(path);
  loaded = { path, selection };
  return selection;
}

/** `file` relative to the repository root, as the selection writes paths. */
export function inRepository(selection: Selection, file: string): string {
  return relative(selection.root ?? "", file)
    .split(sep)
    .join("/");
}

/**
 * Says what a runner does with the selection: one line on stderr, and one
 * record in the file named by `TURBO_TRACE_REPORT`.
 */
export function report(
  selection: Selection,
  runner: string,
  dir: string,
  ran: number,
  total: number
): void {
  process.stderr.write(`turbo trace: running ${ran} of ${total} test files\n`);
  const path = process.env.TURBO_TRACE_REPORT;
  if (!path) {
    return;
  }
  const inside = inRepository(selection, dir);
  const root = selection.root && !inside.startsWith("..") ? inside || "." : dir;
  try {
    appendFileSync(path, `${JSON.stringify({ runner, root, ran, total })}\n`);
  } catch {
    // A report that cannot be written does not fail the tests.
  }
}
