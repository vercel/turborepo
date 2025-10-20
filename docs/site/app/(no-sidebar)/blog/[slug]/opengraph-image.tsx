import React from "react";
import { ImageResponse } from "next/og";
import { notFound } from "next/navigation";

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

// eslint-disable-next-line import/no-default-export -- Nope, we want this.
export default async function Image(props: {
  params: Promise<{ slug?: string }>;
}): Promise<Response> {
  const [geistSemiBold, bg] = await Promise.all([
    fetch(new URL("./Geist-SemiBold.ttf", import.meta.url)).then((res) =>
      res.arrayBuffer()
    ),
    _arrayBufferToBase64(
      await fetch(new URL("./bg.jpg", import.meta.url)).then((res) =>
        res.arrayBuffer()
      )
    ),
  ]);

  const params = await props.params;

  if (!params.slug) {
    notFound();
  }

  let version = "";
  const groups = /^turbo-(?<major>\d+)-(?<minor>\d+)(?:-\d+)*$/.exec(
    params.slug
  );
  if (groups) {
    const { major, minor } = groups.groups as {
      major: string;
      minor: string;
    };
    version = encodeURIComponent(`${major}.${minor}`);
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
          fontWeight: 600,
          fontSize: 60,
          backgroundImage: `url(data:image/jpeg;base64,${bg})`,
          backgroundSize: "1200px 630px",
          color: "#fff",
        }}
      >
        <div
          style={{
            display: "flex",
            fontFamily: "Geist Semibold",
            fontSize: version.length > 3 ? 48 : 52,
            marginTop: "-40",
            marginLeft: "-76",
            fontWeight: "600",
            color: "#fff",
          }}
        >
          {version}
        </div>
      </div>
    ),
    {
      fonts: [
        {
          name: "Geist Semibold",
          data: geistSemiBold,
          weight: 400 as const,
          style: "normal" as const,
        },
      ],
    }
  );
}
