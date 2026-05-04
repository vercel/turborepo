"use server";

import { headers } from "next/headers";
import type { Feedback } from "@/components/geistdocs/feedback";
import { siteId } from "@/geistdocs";
import { checkRateLimit } from "@/lib/rate-limit";
import { getClientIp } from "@/lib/request-ip";
import { emotions } from "./emotions";

const protocol = process.env.NODE_ENV === "production" ? "https" : "http";
const MAX_FEEDBACK_MESSAGE_LENGTH = 2000;
const MAX_FEEDBACK_URL_LENGTH = 2048;
const FEEDBACK_RATE_LIMIT = {
  limit: 5,
  windowSeconds: 60
} as const;

type HeaderList = Awaited<ReturnType<typeof headers>>;

function getBaseUrl(headersList: HeaderList): URL | null {
  const productionUrl = process.env.NEXT_PUBLIC_VERCEL_PROJECT_PRODUCTION_URL;

  if (productionUrl) {
    try {
      return new URL(
        productionUrl.startsWith("http")
          ? productionUrl
          : `${protocol}://${productionUrl}`
      );
    } catch {
      return null;
    }
  }

  if (process.env.VERCEL_ENV === "production") {
    return null;
  }

  const host = headersList.get("host");
  if (!host) {
    return null;
  }

  const forwardedProto = headersList
    .get("x-forwarded-proto")
    ?.split(",")
    .at(0)
    ?.trim();
  const requestProtocol =
    forwardedProto === "http" || forwardedProto === "https"
      ? forwardedProto
      : protocol;

  return new URL(`${requestProtocol}://${host}`);
}

function getValidatedFeedbackUrl(url: string, baseUrl: URL): string | null {
  if (url.length > MAX_FEEDBACK_URL_LENGTH) {
    return null;
  }

  try {
    const feedbackUrl = new URL(url, baseUrl);

    if (feedbackUrl.origin !== baseUrl.origin) {
      return null;
    }

    return feedbackUrl.toString();
  } catch {
    return null;
  }
}

export const sendFeedback = async (
  url: string,
  feedback: Feedback
): Promise<{ success: boolean }> => {
  const headersList = await headers();
  const baseUrl = getBaseUrl(headersList);
  const feedbackUrl =
    baseUrl && typeof url === "string"
      ? getValidatedFeedbackUrl(url, baseUrl)
      : null;
  const candidateFeedback = feedback as Partial<Feedback> | null | undefined;

  if (
    !candidateFeedback ||
    typeof candidateFeedback.message !== "string" ||
    typeof candidateFeedback.emotion !== "string"
  ) {
    return { success: false };
  }

  const emoji = emotions.find((e) => e.name === candidateFeedback.emotion)
    ?.emoji;
  const message = candidateFeedback.message.trim();

  if (
    !feedbackUrl ||
    !emoji ||
    message.length === 0 ||
    message.length > MAX_FEEDBACK_MESSAGE_LENGTH
  ) {
    return { success: false };
  }

  const rateLimit = await checkRateLimit({
    namespace: "feedback",
    key: getClientIp(headersList),
    ...FEEDBACK_RATE_LIMIT
  });

  if (!rateLimit.success) {
    return { success: false };
  }

  try {
    const response = await fetch("https://geistdocs.com/feedback", {
      method: "POST",
      headers: {
        "Content-Type": "application/json"
      },
      body: JSON.stringify({
        note: message,
        url: feedbackUrl,
        emotion: emoji,
        label: siteId
      })
    });

    if (!response.ok) {
      console.error("Feedback request failed:", response.status);

      return { success: false };
    }
  } catch (error) {
    console.error("Feedback request failed:", error);

    return { success: false };
  }

  return {
    success: true
  };
};
