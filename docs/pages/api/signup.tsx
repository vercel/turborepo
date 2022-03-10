import { NextApiRequest, NextApiResponse } from "next";

const TRAY_URL = "https://39dca6c2-9ca4-41b4-82c9-e48202f221f8.trayapp.io";

export default async function handle(
  req: NextApiRequest,
  res: NextApiResponse
) {
  if (req.method === "POST") {
    const user = {
      email: req.body.email,
      campaign_id: req.body.campaignId,
    };

    try {
      await fetch(TRAY_URL, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Accept: "application/json",
        },
        body: JSON.stringify({ user: user }),
      });

      return res.status(201).json(user);
    } catch (error) {
      return res.status(500).json(error);
    }
  } else {
    return res.status(404).send(null);
  }
}
