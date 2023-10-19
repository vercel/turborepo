import { NextApiRequest, NextApiResponse } from "next";
import { withSentry } from "@sentry/nextjs";
import {
  getSubscriber,
  Subscriber,
  updateSubscriber,
} from "../../../lib/ConvertKitApi";

async function handler(req: NextApiRequest, res: NextApiResponse) {
  try {
    if (req.method === "PUT") {
      const subscriber = await updateSubscriber(
        req.query.id as string,
        {
          first_name: req.body.first_name,
          email_address: req.body.email_address,
          fields: req.body.fields,
        } as Subscriber
      );
      res.setHeader("Content-Type", "application/json");
      res.statusCode = 204;
      res.json(subscriber);
    } else {
      const subscriber = await getSubscriber(req.query.id as string);
      res.setHeader("Content-Type", "application/json");
      res.statusCode = 200;
      res.json(subscriber);
    }
  } catch (error) {
    console.log(error);
    res.statusCode = 500;
    res.json({ okay: false });
  }
}

export default withSentry(handler);
