import React from "react";
import { ImageResponse } from "next/og";
import type { NextApiRequest } from "next/index";
import { RepoLogo } from "../../_components/logos/og/repo-logo";
import { VercelLogo } from "../../_components/logos/og/vercel-logo";
import fs from "fs";
import path from "path";

const getBackgroundImage = () => {
  const bgImagePath = path.join(process.cwd(), "app", "api", "og", "bg.jpeg");
  const bgImageBuffer = fs.readFileSync(bgImagePath);
  return `data:image/jpeg;base64,${bgImageBuffer.toString("base64")}`;
};

// Choosing to pull these from node_modules so that its always consistent across the site
// TODO(maybe?): Import them like any other package? import {} from 'geist'
const GEIST_BASE = path.join(
  process.cwd(),
  "node_modules",
  "geist",
  "dist",
  "fonts"
);

const loadFonts = () => {
  const geistSans = fs.readFileSync(
    path.join(GEIST_BASE, "geist-sans", "Geist-Regular.ttf")
  );
  const geistMono = fs.readFileSync(
    path.join(GEIST_BASE, "geist-mono", "GeistMono-Regular.ttf")
  );
  return { geistSans, geistMono };
};

const { geistSans, geistMono } = loadFonts();

export async function GET(req: NextApiRequest): Promise<Response> {
  try {
    if (!req.url) {
      throw new Error("No URL was provided");
    }

    const { searchParams } = new URL(req.url);

    let title: string | null = null;

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
            fontFamily: "Geist Sans",
            fontWeight: 700,
            fontSize: 60,
            backgroundImage: `url(${getBackgroundImage()})`,
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
                fontFamily: "Geist Sans",
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
            data: geistSans,
            weight: 400 as const,
            style: "normal" as const,
          },
        ],
      }
    );
  } catch (err: unknown) {
    // Protects us from serving no image at all if something is broken.
    if (process.env.VERCEL) {
      return new Response(undefined, {
        status: 302,
        headers: {
          Location: "/og-image.png",
        },
      });
    }

    return new Response(undefined, {
      status: 500,
    });
  }
}
