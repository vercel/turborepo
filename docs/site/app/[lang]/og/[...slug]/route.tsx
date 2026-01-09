import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { ImageResponse } from "next/og";
import type { NextRequest } from "next/server";
import { RepoLogo } from "@/components/logos/og/repo-logo";
import { VercelLogo } from "@/components/logos/og/vercel-logo";
import { getPageImage, source } from "@/lib/geistdocs/source";
import { verifyOgSignature } from "@/lib/og/sign";

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

  const [geist, geistMono, backgroundImage] = await Promise.all([
    readFile(join(process.cwd(), "app/[lang]/og/[...slug]/Geist-Regular.ttf")),
    readFile(
      join(process.cwd(), "app/[lang]/og/[...slug]/GeistMono-Regular.ttf")
    ),
    readFile(join(process.cwd(), "app/[lang]/og/[...slug]/bg.jpeg"))
  ]);

  const bg = arrayBufferToBase64(backgroundImage);

  return new ImageResponse(
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        width: "100%",
        height: "100%",
        fontFamily: "Geist Mono",
        fontWeight: 700,
        fontSize: 60,
        backgroundImage: `url(data:image/jpeg;base64,${bg})`,
        backgroundSize: "1200px 630px",
        color: "#fff"
      }}
    >
      <div style={{ display: "flex", height: 97 * 1.1, alignItems: "center" }}>
        <RepoLogo />
      </div>
      {title ? (
        <div
          style={{
            fontFamily: "Geist Mono",
            fontSize: 36,
            letterSpacing: -1.5,
            padding: "40px 20px 30px",
            textAlign: "center",
            backgroundImage: "linear-gradient(to bottom, #fff, #aaa)",
            backgroundClip: "text",
            color: "transparent"
          }}
        >
          {title}
        </div>
      ) : null}
      <div
        style={{
          fontFamily: "Geist Mono",
          fontSize: 18,
          marginTop: 80,
          display: "flex",
          color: "#fff",
          alignItems: "center"
        }}
      >
        <div style={{ marginRight: 12 }}>by</div>
        <VercelLogo fill="white" height={25} />
      </div>
    </div>,
    {
      width: 1200,
      height: 630,
      fonts: [
        {
          name: "Geist Mono",
          data: geistMono,
          weight: 700 as const,
          style: "normal" as const
        },
        {
          name: "Geist Sans",
          data: geist,
          weight: 400 as const,
          style: "normal" as const
        }
      ]
    }
  );
};

export const generateStaticParams = async ({
  params
}: RouteContext<"/[lang]/og/[...slug]">) => {
  const { lang } = await params;

  return source.getPages(lang).map((page) => ({
    lang: page.locale,
    slug: getPageImage(page).segments
  }));
};
