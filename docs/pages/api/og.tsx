import type { ReactElement } from "react";
import React, { createElement } from "react";
import { ImageResponse } from "@vercel/og";
import type { NextApiRequest } from "next/index";
import { PackLogo } from "../../components/logos/og/PackLogo";
import { RepoLogo } from "../../components/logos/og/RepoLogo";
import { TurboLogo } from "../../components/logos/og/TurboLogo";
import { VercelLogo } from "../../components/logos/og/VercelLogo";

function _arrayBufferToBase64(buffer: ArrayBuffer) {
  let binary = "";
  const bytes = new Uint8Array(buffer);
  const len = bytes.byteLength;
  for (let i = 0; i < len; i++) {
    binary += String.fromCharCode(bytes[i]);
  }
  return btoa(binary);
}

async function loadAssets(): Promise<
  [
    { name: string; data: ArrayBuffer; weight: 400 | 700; style: "normal" }[],
    string
  ]
> {
  const [inter, spaceMono, bg] = await Promise.all([
    fetch(
      String(new URL("../../assets/inter-v12-latin-700.ttf", import.meta.url))
    ).then((res) => res.arrayBuffer()),
    fetch(
      String(
        new URL(
          "../../assets/space-mono-v12-latin-regular.ttf",
          import.meta.url
        )
      )
    ).then((res) => res.arrayBuffer()),
    fetch(String(new URL("../../assets/bg.jpeg", import.meta.url))).then(
      (res) => res.arrayBuffer()
    ),
  ]);
  return [
    [
      {
        name: "Inter",
        data: inter,
        weight: 700 as const,
        style: "normal" as const,
      },
      {
        name: "Space Mono",
        data: spaceMono,
        weight: 400 as const,
        style: "normal" as const,
      },
    ],
    _arrayBufferToBase64(bg),
  ];
}

// ?type=<pack|repo>
const TITLE_FOR_TYPE: Record<string, string> = {
  pack: "The successor to Webpack",
  repo: "The build system that makes ship happen",
};

export default async function openGraphImage(
  req: NextApiRequest
): Promise<ImageResponse> {
  try {
    const [fonts, bg] = await loadAssets();

    // eslint-disable-next-line @typescript-eslint/no-non-null-assertion -- It's safe for us to assume that this is coming from `http.Server` here.
    const { searchParams } = new URL(req.url!);

    const type = searchParams.get("type");

    if (!type) {
      throw new Error("No type provided to /api/og.");
    }

    // Start with the default title for the type
    let title = TITLE_FOR_TYPE[type];

    // If there's a ?title=<title> query param, always prefer that.
    if (searchParams.has("title")) {
      // @ts-expect-error -- We just checked .has so we know its there.
      title = searchParams.get("title").slice(0, 100);
    }

    return new ImageResponse(createElement(OGImage, { title, type, bg }), {
      width: 1200,
      height: 630,
      fonts,
    });
  } catch (e: unknown) {
    return new Response(undefined, {
      status: 302,
      headers: {
        Location: "https://turbo.build/og-image.png",
      },
    });
  }
}

export function OGImage({
  title,
  type,
  bg,
}: {
  title: string;
  type: string;
  bg: string;
}): ReactElement {
  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        width: "100%",
        height: "100%",
        fontFamily: "Inter",
        fontWeight: 700,
        fontSize: 60,
        backgroundImage: `url(data:image/jpeg;base64,${bg})`,
        backgroundSize: "1200px 630px",
        color: "#fff",
      }}
    >
      {}
      <div style={{ display: "flex", height: 97 * 1.1, alignItems: "center" }}>
        <Logo type={type} />
      </div>
      {title ? (
        <div
          style={{
            fontFamily: "Space Mono",
            fontSize: 36,
            letterSpacing: -1.5,
            padding: "15px 20px 30px",
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
          fontFamily: "Space Mono",
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
  );
}

function Logo({ type }: { type: string | undefined }): ReactElement {
  if (type === "pack") {
    return <PackLogo height={103 * 1.1} width={697 * 1.1} />;
  }

  if (type === "repo") {
    return <RepoLogo height={83 * 1.1} width={616 * 1.1} />;
  }

  return <TurboLogo height={97 * 1.1} width={459 * 1.1} />;
}

export const config = {
  runtime: "edge",
};
