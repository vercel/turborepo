import React from "react";
import { ImageResponse } from "next/og";
import type { NextApiRequest } from "next/index";
import { RepoLogo } from "../../../_components/logos/og/repo-logo";
import { VercelLogo } from "../../../_components/logos/og/vercel-logo";

export const runtime = "edge";

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
      fetch(new URL("../Geist-Regular.ttf", import.meta.url)).then((res) =>
        res.arrayBuffer()
      ),
      fetch(new URL("../GeistMono-Regular.ttf", import.meta.url)).then((res) =>
        res.arrayBuffer()
      ),
      _arrayBufferToBase64(
        await fetch(new URL("../bg.jpeg", import.meta.url)).then((res) =>
          res.arrayBuffer()
        )
      ),
    ]);

    const reqUrl = req.url || "";
    const { searchParams } = new URL(reqUrl);

    const version = searchParams.get("version") || "1.0.0";
    const title = searchParams.get("title")?.slice(0, 100) || null;

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
          <div
            style={{ display: "flex", height: 97 * 1.1, alignItems: "center" }}
          >
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
                color: "transparent",
              }}
            >
              {title}
            </div>
          ) : null}
          <div
            style={{
              fontFamily: "Geist Mono",
              fontSize: 24,
              marginTop: 20,
              color: "#fff",
              opacity: 0.8,
            }}
          >
            v{version}
          </div>
          <div
            style={{
              fontFamily: "Geist Mono",
              fontSize: 18,
              marginTop: 60,
              display: "flex",
              color: "#fff",
              alignItems: "center",
            }}
          >
            <div style={{ marginRight: 12 }}>by</div>
            <VercelLogo fill="white" height={25} />
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
    if (process.env.VERCEL_ENV === "production") {
      return new Response(undefined, {
        status: 302,
        headers: {
          Location: "https://turborepo.com/og-image.png",
        },
      });
    }

    return new Response(undefined, {
      status: 500,
    });
  }
}