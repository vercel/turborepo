import { mutation, query } from "./_generated/server";
import { v } from "convex/values";

// Define the schema for telegram messages
export const saveMessage = mutation({
  args: {
    messageId: v.number(),
    chatId: v.number(),
    userId: v.optional(v.number()),
    username: v.optional(v.string()),
    firstName: v.optional(v.string()),
    lastName: v.optional(v.string()),
    text: v.string(),
    messageType: v.string(),
    timestamp: v.number(),
    messageThreadId: v.optional(v.number()),
    replyToMessageId: v.optional(v.number()),
  },
  handler: async (ctx, args) => {
    let threadDocId = undefined;

    // If we have a userId, create/update a thread for this user conversation
    if (args.userId) {
      // Check if thread already exists for this user in this chat
      const existingThread = await ctx.db
        .query("telegram_threads")
        .withIndex("by_chat_and_user", (q) =>
          q.eq("chatId", args.chatId).eq("creatorUserId", args.userId!)
        )
        .first();

      if (existingThread) {
        // Update existing thread
        await ctx.db.patch(existingThread._id, {
          lastMessageId: args.messageId,
          lastMessageText: args.text,
          lastMessageTimestamp: args.timestamp,
          messageCount: existingThread.messageCount + 1,
          updatedAt: Date.now(),
        });
        threadDocId = existingThread._id;
      } else {
        // Create new thread for this user
        const threadTitle = args.firstName
          ? `${args.firstName}${args.lastName ? ` ${args.lastName}` : ""}${
              args.username ? ` (@${args.username})` : ""
            }`
          : args.username
          ? `@${args.username}`
          : `User ${args.userId}`;

        threadDocId = await ctx.db.insert("telegram_threads", {
          threadId: args.userId, // Use userId as threadId for user-based threads
          chatId: args.chatId,
          title: threadTitle,
          creatorUserId: args.userId,
          creatorUsername: args.username,
          creatorFirstName: args.firstName,
          creatorLastName: args.lastName,
          firstMessageId: args.messageId,
          lastMessageId: args.messageId,
          lastMessageText: args.text,
          lastMessageTimestamp: args.timestamp,
          messageCount: 1,
          isActive: true,
          createdAt: Date.now(),
          updatedAt: Date.now(),
        });
      }
    }

    // Save the message with threadDocId reference
    const messageDocId = await ctx.db.insert("telegram_messages", {
      messageId: args.messageId,
      chatId: args.chatId,
      userId: args.userId,
      username: args.username,
      firstName: args.firstName,
      lastName: args.lastName,
      text: args.text,
      messageType: args.messageType,
      timestamp: args.timestamp,
      messageThreadId: args.messageThreadId,
      threadDocId: threadDocId, // Link to the user's thread
      createdAt: Date.now(),
    });

    return { success: true, messageId: messageDocId.toString() };
  },
});

// Query to get messages by chat ID
export const getMessagesByChatId = query({
  args: {
    chatId: v.number(),
    limit: v.optional(v.number()),
  },
  handler: async (ctx, args) => {
    const limit = args.limit || 50;
    return await ctx.db
      .query("telegram_messages")
      .filter((q) => q.eq(q.field("chatId"), args.chatId))
      .order("desc")
      .take(limit);
  },
});

// Query to get all messages
export const getAllMessages = query({
  args: {
    limit: v.optional(v.number()),
  },
  handler: async (ctx, args) => {
    const limit = args.limit || 100;
    return await ctx.db.query("telegram_messages").order("desc").take(limit);
  },
});

// Query to get messages by thread
export const getMessagesByThread = query({
  args: {
    chatId: v.number(),
    messageThreadId: v.optional(v.number()),
    limit: v.optional(v.number()),
  },
  handler: async (ctx, args) => {
    const limit = args.limit || 50;
    return await ctx.db
      .query("telegram_messages")
      .withIndex("by_thread", (q) =>
        q.eq("chatId", args.chatId).eq("messageThreadId", args.messageThreadId)
      )
      .order("desc")
      .take(limit);
  },
});

// Query to get messages in a specific thread using the thread document ID
export const getMessagesByThreadDoc = query({
  args: {
    threadDocId: v.id("telegram_threads"),
    limit: v.optional(v.number()),
  },
  handler: async (ctx, args) => {
    const limit = args.limit || 50;
    return await ctx.db
      .query("telegram_messages")
      .withIndex("by_thread_doc", (q) => q.eq("threadDocId", args.threadDocId))
      .order("asc") // Show messages in chronological order within a thread
      .take(limit);
  },
});
