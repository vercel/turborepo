import React from "react";
import Image from "next/image";

type ImageProps = Parameters<typeof Image>[0];

export type ImageFigureProps = Omit<
  ImageProps,
  "caption" | "margin" | "captionSpacing" | "shadow" | "alt" | "borderRadius"
> & {
  caption?: string;
  margin?: number;
  captionSpacing?: number;
  shadow?: boolean;
  alt: string;
  borderRadius?: boolean;
};

export function ImageFigure(props: ImageFigureProps): React.ReactNode {
  const {
    caption,
    margin = 40,
    captionSpacing = null,
    // Destructuring shadow and borderRadius here so
    // they don't get passed to the `...rest` spread in <Image /> below.
    shadow: _unusedShadow = false,
    borderRadius: _unusedBorderRadius = false,
    alt,
    ...rest
  } = props;

  return (
    <figure className="block text-center" style={{ margin: `${margin}px 0` }}>
      <div className="border-box relative inline-block w-full max-w-full overflow-hidden text-[0px]">
        {}
        <Image {...rest} alt={alt} />
      </div>
      {caption ? (
        <figcaption
          className="m-0 text-center text-xs text-gray-500"
          style={captionSpacing ? { marginTop: captionSpacing } : {}}
        >
          {caption}
        </figcaption>
      ) : null}
    </figure>
  );
}
