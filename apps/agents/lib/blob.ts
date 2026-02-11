import { put } from "@vercel/blob";

export async function uploadDiff(
  diff: string,
  branch: string
): Promise<string> {
  const timestamp = new Date().toISOString().replace(/[:.]/g, "-");
  const filename = `diffs/${branch}-${timestamp}.patch`;

  const { pathname } = await put(filename, diff, {
    access: "private",
    contentType: "text/plain"
  });

  return pathname;
}
