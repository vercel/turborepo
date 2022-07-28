import React from "react";
import Image from "next/future/image";

interface ClientProps {
  name: string;
  image: string;
  className?: string;
  theme: string;
  priority: boolean;
  style?: React.CSSProperties;
}

export const Client = React.memo<ClientProps>(
  ({ name, image, style, theme, priority, ...rest }) => (
    <span title={name} {...rest}>
      <Image
        src={image.replace(
          "/logos",
          theme == "dark" ? "/logos/white" : "/logos/color"
        )}
        alt={name}
        width={style?.width ?? 100}
        height={style?.height ?? 75}
        className="inline"
        priority={priority}
        style={{ width: "auto" }}
      />
    </span>
  )
);

Client.displayName = "Client";
