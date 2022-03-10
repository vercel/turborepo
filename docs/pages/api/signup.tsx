import { NextApiRequest, NextApiResponse } from "next";

export default async function handle(
  req: NextApiRequest,
  res: NextApiResponse
) {
  if (req.method === "POST") {
    const user = {
      email: req.body.email,
      campaign_id: process.env.TURBOREPO_SFDC_CAMPAIGN_ID,
    };

    try {
      await fetch(process.env.TRAY_URL, {
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
