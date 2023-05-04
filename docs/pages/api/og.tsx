import React, { createElement } from "react";
import { ImageResponse } from "@vercel/og";

import PackLogo from "../../components/logos/og/PackLogo";
import RepoLogo from "../../components/logos/og/RepoLogo";
import TurboLogo from "../../components/logos/og/TurboLogo";
import VercelLogo from "../../components/logos/og/VercelLogo";

import type { NextApiRequest } from "next/index";

function _arrayBufferToBase64(buffer) {
  var binary = "";
  var bytes = new Uint8Array(buffer);
  var len = bytes.byteLength;
  for (var i = 0; i < len; i++) {
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

export default async function openGraphImage(
  req: NextApiRequest
): Promise<ImageResponse> {
  try {
    const [fonts, bg] = await loadAssets();
    const { searchParams } = new URL(req.url);

    const type = searchParams.get("type");

    // ?title=<title>
    const hasTitle = searchParams.has("title");
    const title = hasTitle
      ? searchParams.get("title")?.slice(0, 100)
      : type === "pack"
      ? "The successor to Webpack"
      : type === "repo"
      ? "The build system that makes ship happen"
      : "";

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
}): JSX.Element {
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
      {/* eslint-disable-next-line  @next/next/no-img-element, jsx-a11y/alt-text */}
      <div style={{ display: "flex", height: 97 * 1.1, alignItems: "center" }}>
        {type === "pack" ? (
          <PackLogo height={103 * 1.1} width={697 * 1.1} />
        ) : type === "repo" ? (
          <RepoLogo height={83 * 1.1} width={616 * 1.1} />
        ) : (
          <TurboLogo height={97 * 1.1} width={459 * 1.1} />
        )}
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

export const config = {
  runtime: "edge",
};
