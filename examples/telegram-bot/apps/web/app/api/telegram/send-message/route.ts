import { NextRequest, NextResponse } from "next/server";
import TelegramBot from "node-telegram-bot-api";

export async function POST(request: NextRequest) {
  console.log("=== TELEGRAM SEND MESSAGE API CALLED ===");
  console.log("Request URL:", request.url);
  console.log("Request method:", request.method);

  try {
    const requestBody = await request.json();
    console.log("Request body:", JSON.stringify(requestBody, null, 2));

    const { chatId, text, messageThreadId } = requestBody;

    // Validate required fields
    if (!chatId || !text) {
      console.log("❌ Validation failed: missing chatId or text");
      return NextResponse.json(
        { error: "chatId and text are required" },
        { status: 400 }
      );
    }

    console.log("✅ Request validation passed");
    console.log("Chat ID:", chatId);
    console.log("Text length:", text.length);
    console.log("Message Thread ID:", messageThreadId || "none");

    // Validate bot token
    const botToken = process.env.TELEGRAM_BOT_TOKEN;
    console.log("Bot token exists:", !!botToken);
    console.log("Bot token length:", botToken ? botToken.length : 0);
    console.log(
      "Bot token preview:",
      botToken ? `${botToken.substring(0, 10)}...` : "undefined"
    );

    if (!botToken) {
      console.error(
        "❌ TELEGRAM_BOT_TOKEN is not defined in environment variables"
      );
      console.error(
        "Available env vars:",
        Object.keys(process.env).filter((key) => key.includes("TELEGRAM"))
      );
      return NextResponse.json(
        { error: "Telegram bot token not configured" },
        { status: 500 }
      );
    }

    // Initialize Telegram Bot
    console.log("🤖 Initializing Telegram Bot...");
    const bot = new TelegramBot(botToken);
    console.log("✅ Telegram Bot initialized");

    // Send message to Telegram
    const telegramOptions: any = {};
    if (messageThreadId) {
      telegramOptions.message_thread_id = messageThreadId;
    }

    console.log("📤 Sending message to Telegram...");
    console.log("Telegram options:", JSON.stringify(telegramOptions, null, 2));

    const telegramResult = await bot.sendMessage(chatId, text, telegramOptions);
    console.log("✅ Message sent to Telegram successfully");
    console.log("Telegram result:", JSON.stringify(telegramResult, null, 2));

    // Save message to Convex database
    const convexUrl = process.env.CONVEX_URL;
    console.log("Convex URL:", convexUrl);

    if (!convexUrl) {
      console.error("❌ CONVEX_URL is not defined in environment variables");
      console.error(
        "Available env vars:",
        Object.keys(process.env).filter((key) => key.includes("CONVEX"))
      );
      return NextResponse.json(
        { error: "Convex URL not configured" },
        { status: 500 }
      );
    }

    console.log("💾 Saving message to Convex database...");
    const convexPayload = {
      messageId: telegramResult.message_id,
      chatId: chatId,
      text: text,
      messageType: "bot_message",
      timestamp: telegramResult.date * 1000,
      messageThreadId: messageThreadId || null,
    };
    console.log("Convex payload:", JSON.stringify(convexPayload, null, 2));

    const saveResponse = await fetch(`${convexUrl}/api/telegram/messages`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
      },
      body: JSON.stringify(convexPayload),
    });

    if (!saveResponse.ok) {
      const errorText = await saveResponse.text();
      console.error("❌ Failed to save message to Convex:", errorText);
      console.error("Response status:", saveResponse.status);
      console.error(
        "Response headers:",
        Object.fromEntries(saveResponse.headers.entries())
      );
      // Still return success since the message was sent to Telegram
    } else {
      console.log("✅ Message saved to Convex successfully");
    }

    const response = {
      success: true,
      telegramMessageId: telegramResult.message_id,
      message: "Message sent successfully",
    };
    console.log("✅ API call completed successfully");
    console.log("Response:", JSON.stringify(response, null, 2));

    return NextResponse.json(response);
  } catch (error: unknown) {
    console.error("❌ ERROR in send-message API:");
    console.error("Error type:", (error as Error)?.constructor?.name);
    console.error("Error message:", (error as Error)?.message);
    console.error("Error stack:", (error as Error)?.stack);
    console.error("Full error object:", error);

    return NextResponse.json(
      {
        error: "Failed to send message",
        details: (error as Error)?.message || "Unknown error",
      },
      { status: 500 }
    );
  }
}
