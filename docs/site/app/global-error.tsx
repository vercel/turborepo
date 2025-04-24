"use client";

import { useEffect } from "react";

interface NextErrorType {
  digest?: string;
  message: string;
  stack?: string;
}

// We use named export as the primary export
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

// To satisfy Next.js's requirement for default export for error pages
// eslint-disable-next-line import/no-default-export -- Required by Next.js for error pages
export default GlobalError;
