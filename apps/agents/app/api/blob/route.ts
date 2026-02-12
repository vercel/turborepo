import { get } from "@vercel/blob";

export const dynamic = "force-dynamic";

export async function GET(request: Request) {
  const { searchParams } = new URL(request.url);
  const url = searchParams.get("url");

  if (!url) {
    return new Response("Missing url parameter", { status: 400 });
  }

  const result = await get(url, { access: "private" });
  if (!result) {
    return new Response("Blob not found", { status: 404 });
  }

  const filename =
    searchParams.get("filename") ??
    decodeURIComponent(url.split("/").pop() ?? "file");

  return new Response(result.stream, {
    headers: {
      "Content-Type": result.blob.contentType,
      "Content-Disposition": `attachment; filename="${filename}"`
    }
  });
}
