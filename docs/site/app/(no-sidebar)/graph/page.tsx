"use client";

import { useSearchParams } from "next/navigation";
import { useEffect, useState } from "react";
import { GraphVisualization } from "#components/graph-visualization.tsx";

export const Page = () => {
  const searchParams = useSearchParams();
  const [decodedPayload, setDecodedPayload] = useState<string | null>(null);

  useEffect(() => {
    const searchParam = searchParams.get("data");

    if (searchParam) {
      try {
        const decoded = atob(searchParam);
        setDecodedPayload(decoded);
      } catch (error) {
        // Silently handle decoding errors
      }
    }
  }, [searchParams]);

  return (
    <>
      <GraphVisualization initialData={decodedPayload} />
    </>
  );
};

export default Page;
