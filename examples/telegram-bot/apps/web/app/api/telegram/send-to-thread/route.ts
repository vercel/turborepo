import { NextRequest, NextResponse } from "next/server";
import TelegramBot from "node-telegram-bot-api";

export async function POST(request: NextRequest) {
  console.log("=== TELEGRAM SEND TO THREAD API CALLED ===");
  console.log("Request URL:", request.url);
  console.log("Request method:", request.method);

  try {
    const requestBody = await request.json();
    console.log("Request body:", JSON.stringify(requestBody, null, 2));

    const { chatId, text, threadDocId, messageThreadId } = requestBody;

    // Validate required fields
    if (!chatId || !text || !threadDocId) {
      console.log("❌ Validation failed: missing required fields");
      return NextResponse.json(
        { error: "chatId, text, and threadDocId are required" },
        { status: 400 }
      );
    }

    console.log("✅ Request validation passed");
    console.log("Chat ID:", chatId);
    console.log("Text length:", text.length);
    console.log("Thread Doc ID:", threadDocId);
    console.log("Message Thread ID:", messageThreadId || "none");

    // Validate bot token
    const botToken = process.env.TELEGRAM_BOT_TOKEN;
    console.log("Bot token exists:", !!botToken);

    if (!botToken) {
      console.error(
        "❌ TELEGRAM_BOT_TOKEN is not defined in environment variables"
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

    // Save message to Convex database with thread linking
    const convexUrl = process.env.CONVEX_URL;
    console.log("Convex URL:", convexUrl);

    if (!convexUrl) {
      console.error("❌ CONVEX_URL is not defined in environment variables");
      return NextResponse.json(
        { error: "Convex URL not configured" },
        { status: 500 }
      );
    }

    console.log("💾 Saving message to Convex database with thread linking...");
    const convexPayload = {
      messageId: telegramResult.message_id,
      chatId: chatId,
      text: text,
      messageType: "bot_message",
      timestamp: telegramResult.date * 1000,
      messageThreadId: messageThreadId || null,
      threadDocId: threadDocId, // Link to specific thread
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
      // Still return success since the message was sent to Telegram
    } else {
      console.log("✅ Message saved to Convex successfully");
    }

    const response = {
      success: true,
      telegramMessageId: telegramResult.message_id,
      message: "Message sent to thread successfully",
    };
    console.log("✅ API call completed successfully");
    console.log("Response:", JSON.stringify(response, null, 2));

    return NextResponse.json(response);
  } catch (error: unknown) {
    console.error("❌ ERROR in send-to-thread API:");
    console.error("Error type:", (error as Error)?.constructor?.name);
    console.error("Error message:", (error as Error)?.message);
    console.error("Error stack:", (error as Error)?.stack);
    console.error("Full error object:", error);

    return NextResponse.json(
      {
        error: "Failed to send message to thread",
        details: (error as Error)?.message || "Unknown error",
      },
      { status: 500 }
    );
  }
}
