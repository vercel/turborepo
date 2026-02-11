import { put } from "@vercel/blob";

export async function uploadDiff(
  diff: string,
  branch: string
): Promise<string> {
  const timestamp = new Date().toISOString().replace(/[:.]/g, "-");
  const filename = `diffs/${branch}-${timestamp}.patch`;

  const { url } = await put(filename, diff, {
    access: "public",
    contentType: "text/plain"
  });

  return url;
}
