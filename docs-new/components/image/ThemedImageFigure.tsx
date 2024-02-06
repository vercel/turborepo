import React from "react";
import cn from "classnames";
import type { ImageFigureProps } from "./ImageFigure";
import type { ThemedImageProps } from "./ThemedImage";
import { ThemedImage } from "./ThemedImage";

export type ThemedImageFigureProps = Omit<ImageFigureProps, "src"> &
  ThemedImageProps;

export function ThemedImageFigure(
  props: ThemedImageFigureProps
): React.ReactNode {
  const {
    caption,
    margin = 40,
    captionSpacing = null,
    shadow = false,
    borderRadius = false,
    ...rest
  } = props;

  return (
    <figure
      className="block -mx-4 text-center sm:-mx-4 md:-mx-7 lg:-mx-12"
      style={{ marginTop: `${margin}px`, marginBottom: `${margin}px` }}
    >
      <div
        className={cn(
          "relative inline-block max-w-full overflow-hidden border-box text-[0px]",
          {
            "rounded-md": borderRadius,
            "shadow-lg": shadow,
          }
        )}
      >
        {}
        <ThemedImage {...rest} />
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
