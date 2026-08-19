import { ImageResponse } from "next/og";
import type { NextRequest } from "next/server";
import { verifyOgSignatureEdge } from "@/lib/og/sign-edge";

export const runtime = "edge";

const OG_BACKGROUND_URL =
  "https://ufa25dqjajkmio0q.public.blob.vercel-storage.com/docs-og-background.png";

function arrayBufferToBase64(buffer: ArrayBuffer): string {
  let binary = "";
  const bytes = new Uint8Array(buffer);
  const len = bytes.byteLength;
  for (let i = 0; i < len; i++) {
    binary += String.fromCharCode(bytes[i]);
  }
  return btoa(binary);
}

export async function GET(req: NextRequest): Promise<Response> {
  try {
    const { searchParams } = new URL(req.url);

    const title = searchParams.get("title") || "";
    const section = searchParams.get("section") || "Turborepo";
    const sig = searchParams.get("sig") || "";

    // Verify signature - title can be empty for home page
    const isValid = await verifyOgSignatureEdge(
      searchParams.has("section") ? { title, section } : { title },
      sig
    );
    if (!isValid) {
      return new Response("Unauthorized", { status: 401 });
    }

    const [geist, geistMono, bg] = await Promise.all([
      fetch(new URL("./Geist-Regular.ttf", import.meta.url)).then((res) =>
        res.arrayBuffer()
      ),
      fetch(new URL("./GeistMono-Regular.ttf", import.meta.url)).then((res) =>
        res.arrayBuffer()
      ),
      arrayBufferToBase64(
        await fetch(OG_BACKGROUND_URL).then((res) => {
          if (!res.ok) {
            throw new Error(`Failed to load OG background: ${res.status}`);
          }
          return res.arrayBuffer();
        })
      )
    ]);

    return new ImageResponse(
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
          color: "#fff"
        }}
      >
        <div
          style={{
            position: "absolute",
            top: 198,
            left: 55,
            display: "flex",
            flexDirection: "column"
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
                color: "#fff"
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
              color: "#888"
            }}
          >
            {section}
          </div>
        </div>
      </div>,
      {
        width: 1200,
        height: 630,
        fonts: [
          {
            name: "Geist Mono",
            data: geistMono,
            weight: 400 as const,
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
  } catch (err: unknown) {
    // Prevents us from having no OG image at all in production.
    if (process.env.VERCEL_ENV === "production") {
      return new Response(undefined, {
        status: 302,
        headers: {
          Location: "https://turborepo.dev/og-image.png"
        }
      });
    }

    // We want to see the 500s everywhere else.
    return new Response(undefined, {
      status: 500
    });
  }
}
