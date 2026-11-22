import { appendFileSync, existsSync, readFileSync } from "node:fs";
import { dirname, join, relative, sep } from "node:path";

/** The selection of `turbo trace affected --for-runners`, as a plugin uses it. */
export interface Selection {
  /** The repository root: the selection's paths are relative to it. */
  root: string;
  /** The test files the trace knows. */
  known: string[];
  /** Those of them that the change cannot affect: the only files dropped. */
  dropped: Set<string>;
}

let loaded: { path: string; selection?: Selection } | undefined;

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

function warn(why: string): undefined {
  process.stderr.write(`turbo trace: ${why}: running every test\n`);
}

function read(path: string): Selection | undefined {
  let doc: Record<string, unknown>;
  try {
    doc = JSON.parse(readFileSync(path, "utf8")) ?? {};
  } catch (error) {
    return warn(`cannot read the selection ${path} (${error})`);
  }
  const lists = [doc.files, doc.runWhole, doc.unselected];
  if (doc.version !== 1 || !lists.every(strings)) {
    return warn(`the selection ${path} is not a version 1 document`);
  }
  const [files, runWhole, unselected] = lists;
  const known = [...files, ...unselected];
  const root = findRoot(known[0] ?? "");
  if (root === undefined) {
    return warn(`the files of the selection ${path} are not around here`);
  }
  // An empty `runWhole` entry is the root package: it holds every file.
  const whole = (file: string) =>
    doc.runAll !== false || runWhole.some((dir) => file.startsWith(dir));
  const dropped = new Set(unselected.filter((file) => !whole(file)));
  return { root, known, dropped };
}

/**
 * The selection named by `TURBO_TRACE_SELECTION`. There is none without the
 * variable, and none, with one warning, when the document cannot be used.
 */
export function loadSelection(): Selection | undefined {
  const path = process.env.TURBO_TRACE_SELECTION;
  if (path && loaded?.path !== path) {
    loaded = { path, selection: read(path) };
  }
  return path ? loaded?.selection : undefined;
}

/** `file` relative to the repository root, as the selection writes paths. */
export function inRepository(selection: Selection, file: string): string {
  return relative(selection.root, file).split(sep).join("/");
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
  const root = inRepository(selection, dir) || ".";
  try {
    appendFileSync(path, `${JSON.stringify({ runner, root, ran, total })}\n`);
  } catch {
    // A report that cannot be written does not fail the tests.
  }
}
