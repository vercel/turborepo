import { ImageResponse } from "next/og";
import type { NextRequest } from "next/server";
import { RepoLogo } from "@/components/logos/og/repo-logo";
import { verifyOgSignatureEdge } from "@/lib/og/sign-edge";

export const runtime = "edge";

export async function GET(req: NextRequest): Promise<Response> {
  try {
    const { searchParams } = new URL(req.url);

    const version = searchParams.get("version") || "";
    const sig = searchParams.get("sig") || "";

    // Verify signature
    const isValid = await verifyOgSignatureEdge({ version }, sig);
    if (!isValid) {
      return new Response("Unauthorized", { status: 401 });
    }

    const geistSans = await fetch(
      new URL("../Geist-Regular.ttf", import.meta.url)
    ).then((res) => res.arrayBuffer());

    return new ImageResponse(
      (
        <div
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            width: "100%",
            height: "100%",
            background: "#000",
            color: "#fff",
          }}
        >
          <div
            style={{
              display: "flex",
              alignItems: "center",
              transform: "translateY(-29px)",
            }}
          >
            <RepoLogo height={88} width={653} />
            <div
              style={{
                display: "flex",
                alignItems: "center",
                justifyContent: "center",
                marginLeft: 42,
                padding: "9px 14px",
                border: "1.3px solid #fff",
                borderRadius: 7,
                fontFamily: "Geist Sans",
                fontSize: 68,
                fontWeight: 450,
                letterSpacing: "-3px",
                lineHeight: 0.9,
                transform: "translateY(3.5px)",
              }}
            >
              <div style={{ display: "flex", transform: "translateY(0.5px)" }}>
                {version}
              </div>
            </div>
          </div>
        </div>
      ),
      {
        width: 1200,
        height: 630,
        fonts: [
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
    if (process.env.VERCEL_ENV === "production") {
      return new Response(undefined, {
        status: 302,
        headers: {
          Location: "https://turborepo.dev/og-image.png",
        },
      });
    }

    return new Response(undefined, {
      status: 500,
    });
  }
}
