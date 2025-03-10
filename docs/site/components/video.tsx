import type { JSX } from "react";

export function Video(
  props: React.HTMLProps<HTMLVideoElement> & {
    width: number;
    height: number;
    caption?: string;
  }
): JSX.Element {
  return (
    <figure>
      <video
        autoPlay
        loop
        muted
        playsInline
        {...props}
        style={{
          aspectRatio: props.width / props.height,
          margin: 0,
        }}
      />
      <figcaption>{props.caption}</figcaption>
    </figure>
  );
}
