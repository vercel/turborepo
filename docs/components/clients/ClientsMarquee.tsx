import React from "react";
import { Client } from "./Client";
import { users } from "./users";

const pinnedLogos = users.filter((p) => p.pinned);

export const ClientsMarquee = React.memo((props) => {
  return (
    <div className="overflow-x-hidden">
      <div className="relative" {...props}>
        <div className="inline-block wrapper">
          {pinnedLogos.map(({ caption, infoLink, image, style }) => (
            <Client
              className="mx-8 align-middle opacity-50"
              key={caption}
              style={style}
              name={caption}
              image={image}
            />
          ))}
        </div>
      </div>
    </div>
  );
});

ClientsMarquee.displayName = "ClientsMarquee";
