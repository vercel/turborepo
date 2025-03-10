import React from "react";
import { ImageResponse } from "next/og";
import type { NextApiRequest } from "next/index";
import { PRODUCT_SLOGANS } from "@/lib/constants";
import { RepoLogo } from "../../_components/logos/og/repo-logo";
import { TurboLogo } from "../../_components/logos/og/turbo-logo";
import { VercelLogo } from "../../_components/logos/og/vercel-logo";

export type Products = "repo";

function _arrayBufferToBase64(buffer: ArrayBuffer): string {
  let binary = "";
  const bytes = new Uint8Array(buffer);
  const len = bytes.byteLength;
  for (let i = 0; i < len; i++) {
    binary += String.fromCharCode(bytes[i]);
  }
  return btoa(binary);
}

export async function GET(req: NextApiRequest): Promise<Response> {
  try {
    const [geist, geistMono, bg] = await Promise.all([
      fetch(new URL("./Geist-Regular.ttf", import.meta.url)).then((res) =>
        res.arrayBuffer()
      ),
      fetch(new URL("./GeistMono-Regular.ttf", import.meta.url)).then((res) =>
        res.arrayBuffer()
      ),
      _arrayBufferToBase64(
        await fetch(new URL("./bg.jpeg", import.meta.url)).then((res) =>
          res.arrayBuffer()
        )
      ),
    ]);

    if (!req.url) {
      throw new Error("No URL was provided");
    }

    const { searchParams } = new URL(req.url);

    let title: string | null = null;

    const type: string | null = searchParams.get("type");
    const typeIsNotProduct = type !== "repo";
    if (typeIsNotProduct && !searchParams.has("title")) {
      throw new Error(type ?? "undefined");
    }

    if (!typeIsNotProduct) {
      title = PRODUCT_SLOGANS[type as keyof typeof PRODUCT_SLOGANS];
    }

    if (searchParams.has("title")) {
      // @ts-expect-error -- We just checked .has so we know its there.
      title = searchParams.get("title").slice(0, 100);
    }

    return new ImageResponse(
      (
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
            color: "#fff",
          }}
        >
          {}
          <div
            style={{ display: "flex", height: 97 * 1.1, alignItems: "center" }}
          >
            <Logo type={type as Products | null} />
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
                color: "transparent",
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
              alignItems: "center",
            }}
          >
            <div style={{ marginRight: 12 }}>by</div>
            <VercelLogo fill="white" height={30} />
          </div>
        </div>
      ),
      {
        fonts: [
          {
            name: "Geist Mono",
            data: geistMono,
            weight: 700 as const,
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
  } catch (err: unknown) {
    if (process.env.NODE_ENV === "development") {
      // eslint-disable-next-line no-console
      console.log(
        // @ts-expect-error -- Error handling.
        `Incorrect type provided to /api/og. "${err.message}" was provided.`
      );
    }
    return new Response(undefined, {
      status: 302,
      headers: {
        Location: "https://turbo.build/og-image.png",
      },
    });
  }
}

function Logo({ type }: { type: Products | null }): JSX.Element {
  if (type === "repo") {
    return <RepoLogo height={83 * 1.1} width={616 * 1.1} />;
  }

  return <TurboLogo height={97 * 1.1} width={459 * 1.1} />;
}
