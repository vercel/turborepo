import { list } from "@vercel/blob";
import Link from "next/link";

export const dynamic = "force-dynamic";

export default async function DiffsPage() {
  const { blobs } = await list({ prefix: "diffs/" });

  return (
    <main className="mx-auto max-w-3xl px-6 py-16 font-mono">
      <div className="mb-8 flex items-center justify-between">
        <h1 className="text-2xl font-bold">Audit Diffs</h1>
        <Link href="/" className="text-sm text-neutral-500 hover:text-white">
          Back
        </Link>
      </div>

      {blobs.length === 0 ? (
        <p className="text-neutral-500">No diffs yet.</p>
      ) : (
        <ul className="space-y-3">
          {blobs
            .sort(
              (a, b) =>
                new Date(b.uploadedAt).getTime() -
                new Date(a.uploadedAt).getTime()
            )
            .map((blob) => {
              const name = blob.pathname.replace("diffs/", "");
              return (
                <li
                  key={blob.url}
                  className="flex items-center justify-between rounded border border-neutral-800 p-4"
                >
                  <div>
                    <p className="text-sm font-medium">{name}</p>
                    <p className="text-xs text-neutral-500">
                      {new Date(blob.uploadedAt).toLocaleString()} ·{" "}
                      {(blob.size / 1024).toFixed(1)} KB
                    </p>
                  </div>
                  <div className="flex gap-3">
                    <Link
                      href={`/vuln-diffs/view?pathname=${encodeURIComponent(blob.pathname)}`}
                      className="rounded border border-neutral-300 bg-white px-3 py-1.5 text-xs text-neutral-700 hover:bg-neutral-100 dark:border-neutral-800 dark:bg-neutral-800 dark:text-neutral-200 dark:hover:bg-neutral-700"
                    >
                      View
                    </Link>
                  </div>
                </li>
              );
            })}
        </ul>
      )}
    </main>
  );
}
