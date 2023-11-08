import React from "react";
import Image from "next/image";

type ImageProps = Parameters<typeof Image>[0];

export type ImageFigureProps = ImageProps & {
  caption?: string;
  margin?: number;
  captionSpacing?: number;
  shadow?: boolean;
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
    ...rest
  } = props;

  return (
    <figure className="block text-center" style={{ margin: `${margin}px 0` }}>
      <div className="relative inline-block w-full max-w-full overflow-hidden border-box text-[0px]">
        {}
        <Image {...rest} />
      </div>
      {caption ? (
        <figcaption
          className="m-0 text-xs text-center text-gray-500"
          style={captionSpacing ? { marginTop: captionSpacing } : {}}
        >
          {caption}
        </figcaption>
      ) : null}
    </figure>
  );
}
