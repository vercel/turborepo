import { get } from "@vercel/blob";
import Link from "next/link";
import { CopyButton } from "./copy-button";

export const dynamic = "force-dynamic";

function parseDiffLines(diff: string) {
  return diff.split("\n").map((line, i) => {
    let className = "text-neutral-400";
    if (line.startsWith("+") && !line.startsWith("+++")) {
      className = "text-green-400 bg-green-950/30";
    } else if (line.startsWith("-") && !line.startsWith("---")) {
      className = "text-red-400 bg-red-950/30";
    } else if (line.startsWith("@@")) {
      className = "text-blue-400";
    } else if (line.startsWith("diff --git")) {
      className = "text-yellow-400 font-bold";
    }
    return (
      <div key={i} className={`${className} whitespace-pre px-4`}>
        {line || " "}
      </div>
    );
  });
}

async function readBlobText(pathname: string): Promise<string | null> {
  const result = await get(pathname, { access: "private" });
  if (!result) return null;
  const reader = result.stream.getReader();
  const chunks: Uint8Array[] = [];
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    chunks.push(value);
  }
  const combined = new Uint8Array(chunks.reduce((acc, c) => acc + c.length, 0));
  let offset = 0;
  for (const chunk of chunks) {
    combined.set(chunk, offset);
    offset += chunk.length;
  }
  return new TextDecoder().decode(combined);
}

export default async function ViewDiffPage({
  searchParams
}: {
  searchParams: Promise<{ pathname?: string }>;
}) {
  const { pathname } = await searchParams;

  if (!pathname) {
    return (
      <main className="mx-auto max-w-3xl px-6 py-16 font-mono">
        <p className="text-red-500">Missing pathname parameter.</p>
      </main>
    );
  }

  const diff = await readBlobText(pathname);
  if (!diff) {
    return (
      <main className="mx-auto max-w-3xl px-6 py-16 font-mono">
        <p className="text-red-500">Failed to fetch diff.</p>
      </main>
    );
  }

  const filename = pathname.split("/").pop() ?? "diff.patch";

  return (
    <main className="mx-auto max-w-6xl px-6 py-16 font-mono">
      <div className="mb-6 flex items-center justify-between">
        <div>
          <h1 className="text-xl font-bold">{filename}</h1>
          <p className="text-xs text-neutral-500">
            {(diff.length / 1024).toFixed(1)} KB · {diff.split("\n").length}{" "}
            lines
          </p>
        </div>
        <div className="flex gap-2">
          <CopyButton text={diff} />
          <Link
            href="/vuln-diffs"
            className="rounded border border-neutral-300 bg-white px-3 py-1.5 text-xs text-neutral-700 hover:bg-neutral-100 dark:border-neutral-800 dark:bg-neutral-800 dark:text-neutral-200 dark:hover:bg-neutral-700"
          >
            Back to list
          </Link>
        </div>
      </div>

      <div className="overflow-x-auto rounded border border-neutral-800 bg-neutral-950 py-2 text-xs leading-5">
        {parseDiffLines(diff)}
      </div>
    </main>
  );
}
