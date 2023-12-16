import type { NextApiRequest, NextApiResponse } from "next";
import { wrapApiHandlerWithSentry } from "@sentry/nextjs";

const CAMPAIGN_ID = process.env.TURBOREPO_SFDC_CAMPAIGN_ID;
const TRAY_URL = process.env.TRAY_URL;

async function handler(req: NextApiRequest, res: NextApiResponse) {
  if (req.method === "POST") {
    const reqBody = req.body as { email?: string };

    if (!reqBody.email) {
      throw new Error("No email was provided.");
    }

    const user = {
      email: reqBody.email,
      campaign_id: CAMPAIGN_ID,
    };

    if (!TRAY_URL) {
      throw new Error("No TRAY_URL was provided.");
    }

    try {
      await fetch(TRAY_URL, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Accept: "application/json",
        },
        body: JSON.stringify({ user }),
      });

      res.status(201).json(user);
    } catch (error) {
      res.status(500).json(error);
    }
  } else {
    res.status(404).send(null);
  }
}

export default wrapApiHandlerWithSentry(handler, "/api/signup");
