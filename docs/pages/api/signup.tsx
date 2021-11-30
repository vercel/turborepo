import { NextApiRequest, NextApiResponse } from "next";
import { subscribeToForm } from "../../lib/ConvertKitApi";

const FORM_ID = process.env.CONVERTKIT_FORM_ID;

export default async function handle(
  req: NextApiRequest,
  res: NextApiResponse
) {
  if (req.method === "POST") {
    const subscriber = await subscribeToForm({
      formId: FORM_ID,
      email: req.body.email,
      firstName: req.body.firstName,
      fields: {
        last_name: req.body.lastName,
      },
    });

    return res.status(201).json(subscriber);
  } else {
    return res.status(404).send(null);
  }
}
