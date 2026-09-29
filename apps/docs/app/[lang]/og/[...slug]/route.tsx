import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { findPath } from "fumadocs-core/page-tree";
import { ImageResponse } from "next/og";
import type { NextRequest } from "next/server";
import { getPageImage, source } from "@/lib/geistdocs/source";
import { verifyOgSignature } from "@/lib/og/sign";

const DOCS_OG_BACKGROUND_URL =
  "https://ufa25dqjajkmio0q.public.blob.vercel-storage.com/docs-og-background.png";

function arrayBufferToBase64(buffer: ArrayBuffer): string {
  let binary = "";
  const bytes = new Uint8Array(buffer);
  const len = bytes.byteLength;
  for (let i = 0; i < len; i++) {
    binary += String.fromCharCode(bytes[i]);
  }
  return Buffer.from(binary, "binary").toString("base64");
}

export const GET = async (
  request: NextRequest,
  { params }: RouteContext<"/[lang]/og/[...slug]">
) => {
  const { slug, lang } = await params;

  // Verify signature
  const { searchParams } = new URL(request.url);
  const sig = searchParams.get("sig");
  const path = slug.join("/");

  if (!sig || !verifyOgSignature({ path }, sig)) {
    return new Response("Unauthorized", { status: 401 });
  }

  const page = source.getPage(slug.slice(0, -1), lang);

  if (!page) {
    return new Response("Not found", { status: 404 });
  }

  const { title } = page.data;
  const pageTreePath = findPath(
    source.pageTree[lang].children,
    (node) => node.type === "page" && node.url === page.url
  );
  const section =
    pageTreePath?.find((node) => node.type === "folder")?.name ?? "Docs";

  const [geist, geistMono, backgroundImage] = await Promise.all([
    readFile(join(process.cwd(), "app/[lang]/og/[...slug]/Geist-Regular.ttf")),
    readFile(
      join(process.cwd(), "app/[lang]/og/[...slug]/GeistMono-Regular.ttf")
    ),
    fetch(DOCS_OG_BACKGROUND_URL).then((response) => {
      if (!response.ok) {
        throw new Error(
          `Failed to load docs OG background: ${response.status}`
        );
      }

      return response.arrayBuffer();
    }),
  ]);

  const bg = arrayBufferToBase64(backgroundImage);

  return new ImageResponse(
    (
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          position: "relative",
          width: "100%",
          height: "100%",
          fontFamily: "Geist Sans",
          backgroundImage: `url(data:image/png;base64,${bg})`,
          backgroundPosition: "center",
          backgroundSize: "cover",
          color: "#fff",
        }}
      >
        <div
          style={{
            position: "absolute",
            top: 198,
            left: 55,
            display: "flex",
            flexDirection: "column",
          }}
        >
          {title ? (
            <div
              style={{
                width: 550,
                fontSize: 64,
                fontWeight: 400,
                letterSpacing: -2.7,
                lineHeight: 1,
                color: "#fff",
              }}
            >
              {title}
            </div>
          ) : null}
          <div
            style={{
              marginTop: 32,
              fontFamily: "Geist Mono",
              fontSize: 28,
              fontWeight: 400,
              letterSpacing: -1,
              lineHeight: 1,
              color: "#888",
            }}
          >
            {section}
          </div>
        </div>
      </div>
    ),
    {
      width: 1200,
      height: 630,
      fonts: [
        {
          name: "Geist Mono",
          data: geistMono,
          weight: 400 as const,
          style: "normal" as const,
        },
        {
          name: "Geist Sans",
          data: geist,
          weight: 400 as const,
          style: "normal" as const,
        },
      ],
    }
  );
};

export const generateStaticParams = async ({
  params,
}: RouteContext<"/[lang]/og/[...slug]">) => {
  const { lang } = await params;

  return source.getPages(lang).map((page) => ({
    lang: page.locale,
    slug: getPageImage(page).segments,
  }));
};
