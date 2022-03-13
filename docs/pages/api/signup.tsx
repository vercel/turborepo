import { NextApiRequest, NextApiResponse } from "next";

const CAMPAIGN_ID = process.env.TURBOREPO_SFDC_CAMPAIGN_ID;
const TRAY_URL = process.env.TRAY_URL;

export default async function handle(
  req: NextApiRequest,
  res: NextApiResponse
) {
  if (req.method === "POST") {
    const user = {
      email: req.body.email,
      campaign_id: CAMPAIGN_ID,
    };

    try {
      const trayRes = await fetch(TRAY_URL, {
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
