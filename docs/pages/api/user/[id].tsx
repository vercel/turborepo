import type { NextApiRequest, NextApiResponse } from "next";
import { wrapApiHandlerWithSentry } from "@sentry/nextjs";
import type { Subscriber } from "../../../lib/ConvertKitApi";
import { getSubscriber, updateSubscriber } from "../../../lib/ConvertKitApi";

interface RequestBody {
  first_name?: string;
  email_address?: string;
  fields?: Record<string, unknown>;
}

async function handler(req: NextApiRequest, res: NextApiResponse) {
  try {
    if (req.method === "PUT") {
      if (!req.body) {
        throw new Error("No body was provided.");
      }

      const reqBody = req.body as RequestBody;

      if (!reqBody.first_name) {
        throw new Error("First name was not provided.");
      }

      if (!reqBody.email_address) {
        throw new Error("Email address was not provided.");
      }

      if (!reqBody.fields) {
        throw new Error("No fields were provided.");
      }

      const subscriber = await updateSubscriber(
        req.query.id as string,
        {
          first_name: reqBody.first_name,
          email_address: reqBody.email_address,
          fields: reqBody.fields,
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
  } catch (error: unknown) {
    if (typeof error === "string") {
      throw new Error(error);
    } else if (error instanceof Error) {
      throw new Error(error.message);
    }
    res.statusCode = 500;
    res.json({ okay: false });
  }
}

export default wrapApiHandlerWithSentry(handler, "./api/user/:id");
