import React from "react";
import { ImageFigureProps } from "./ImageFigure";
import { ThemedImage, ThemedImageProps } from "./ThemedImage";
import cn from "classnames";
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
        {/* eslint-disable-next-line jsx-a11y/alt-text */}
        <ThemedImage {...rest} />
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
