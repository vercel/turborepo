"use client";

import { useEffect } from "react";

interface NextErrorType {
  digest?: string;
  message: string;
  stack?: string;
}

export function GlobalError({ error }: { error: NextErrorType }): JSX.Element {
  useEffect(() => {
    // eslint-disable-next-line no-console -- This console log is intentional for error reporting
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

// Next.js requires a default export for error pages
export default GlobalError;
