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
    shadow = false,
    borderRadius = false,
    ...rest
  } = props;

  return (
    <figure className="block text-center" style={{ margin: `${margin}px 0` }}>
      <div className="relative inline-block w-full max-w-full overflow-hidden border-box text-[0px]">
        {/* eslint-disable-next-line jsx-a11y/alt-text */}
        <Image {...rest} />
      </div>
      {caption && (
        <figcaption
          className="m-0 text-xs text-center text-gray-500"
          style={captionSpacing ? { marginTop: captionSpacing } : {}}
        >
          {caption}
        </figcaption>
      )}
    </figure>
  );
}
