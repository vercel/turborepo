"use client";

import type Error from "next/error";
import { useEffect } from "react";

export default function GlobalError({
  error,
}: {
  error: Error & { digest?: string };
}): JSX.Element {
  useEffect(() => {
    // eslint-disable-next-line no-console
    console.log(error);
  }, [error]);

  return (
    <html lang="en">
      <body>
        <h2>Something went wrong!</h2>
      </body>
    </html>
  );
}
