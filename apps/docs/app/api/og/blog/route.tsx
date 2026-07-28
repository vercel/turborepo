import { ImageResponse } from "next/og";
import type { NextRequest } from "next/server";
import { verifyOgSignatureEdge } from "@/lib/og/sign-edge";

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

    const [geistSemiBold, bg] = await Promise.all([
      fetch(new URL("./Geist-SemiBold.ttf", import.meta.url)).then((res) =>
        res.arrayBuffer()
      ),
      _arrayBufferToBase64(
        await fetch(new URL("./bg.jpg", import.meta.url)).then((res) =>
          res.arrayBuffer()
        )
      )
    ]);

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
          fontWeight: 600,
          fontSize: 60,
          backgroundImage: `url(data:image/jpeg;base64,${bg})`,
          backgroundSize: "1200px 630px",
          color: "#fff"
        }}
      >
        <div
          style={{
            display: "flex",
            fontFamily: "Geist Semibold",
            fontSize: 52,
            marginTop: "-40",
            marginLeft: "-76",
            fontWeight: "600",
            color: "#fff"
          }}
        >
          {version}
        </div>
      </div>,
      {
        fonts: [
          {
            name: "Geist Semibold",
            data: geistSemiBold,
            weight: 400 as const,
            style: "normal" as const
          }
        ]
      }
    );
  } catch (err: unknown) {
    if (process.env.VERCEL_ENV === "production") {
      return new Response(undefined, {
        status: 302,
        headers: {
          Location: "https://turborepo.dev/og-image.png"
        }
      });
    }

    return new Response(undefined, {
      status: 500
    });
  }
}
