import { httpRouter } from "convex/server";
import { httpAction } from "./_generated/server";
import { api } from "./_generated/api";
import { saveMessageAPI, getMessagesAPI, saveMessageToThreadAPI } from "./api";

const http = httpRouter();

// Telegram bot API endpoints
http.route({
  path: "/api/telegram/messages",
  method: "POST",
  handler: saveMessageAPI,
});

http.route({
  path: "/api/telegram/messages",
  method: "GET",
  handler: getMessagesAPI,
});

// New endpoint for saving messages to specific threads
http.route({
  path: "/api/telegram/messages/thread",
  method: "POST",
  handler: saveMessageToThreadAPI,
});

// Health check endpoint
const healthCheck = httpAction(async () => {
  return new Response(
    JSON.stringify({
      status: "healthy",
      timestamp: new Date().toISOString(),
      service: "convex-telegram-backend",
    }),
    {
      status: 200,
      headers: { "Content-Type": "application/json" },
    }
  );
});

http.route({
  path: "/api/health",
  method: "GET",
  handler: healthCheck,
});

// Note: Message sending is now handled directly by Next.js app using node-telegram-bot-api
// The Next.js app will send messages to Telegram and then save them to the database

export default http;
