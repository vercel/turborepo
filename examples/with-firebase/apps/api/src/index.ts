import express from "express";
import * as functions from "firebase-functions";
import { exampleConfigFromShared } from "shared/util";

const app = express();

app.get("*", (req, res) => {
  res.send(exampleConfigFromShared);
});

export const server = functions.https.onRequest(app);
