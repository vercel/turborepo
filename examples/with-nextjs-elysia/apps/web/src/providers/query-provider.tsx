"use client";

import {
  MutationCache,
  QueryCache,
  QueryClient,
  QueryClientProvider,
} from "@tanstack/react-query";
import type { ReactNode } from "react";
import { useState } from "react";
import { Toaster, toast } from "sonner";

/**
 * RFC 9457 problem details returned by the API
 */
interface ProblemDetails {
  detail?: string;
  instance?: string;
  status: number;
  title: string;
  [key: string]: unknown;
}

const isProblemDetails = (value: unknown): value is ProblemDetails =>
  typeof value === "object" && value !== null && "title" in value;

const describeError = (error: unknown) => {
  // Eden Treaty errors are shaped like `{ status, value: ProblemDetails }`
  const problem =
    typeof error === "object" && error !== null && "value" in error
      ? error.value
      : error;

  if (isProblemDetails(problem)) {
    return { title: problem.title, description: problem.detail };
  }

  return {
    title: "Request failed",
    description:
      error instanceof Error ? error.message : "An unknown error occurred",
  };
};

// Shared error handler for every query and mutation
const handleGlobalError = (error: unknown) => {
  const { title, description } = describeError(error);
  toast.error(title, { description, duration: 4000 });
};

export default function QueryProvider({ children }: { children: ReactNode }) {
  const [queryClient] = useState(
    () =>
      new QueryClient({
        defaultOptions: {
          queries: {
            refetchOnWindowFocus: false,
            retry: 1,
          },
        },
        queryCache: new QueryCache({ onError: handleGlobalError }),
        mutationCache: new MutationCache({ onError: handleGlobalError }),
      })
  );

  return (
    <QueryClientProvider client={queryClient}>
      {children}
      <Toaster />
    </QueryClientProvider>
  );
}
