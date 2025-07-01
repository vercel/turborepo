import { mutation } from "./_generated/server";
import { v } from "convex/values";

// Mutation to save a message directly to a specific thread
export const saveMessageToThread = mutation({
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
    threadDocId: v.id("telegram_threads"), // Required thread document ID
    replyToMessageId: v.optional(v.number()),
  },
  handler: async (ctx, args) => {
    // Verify the thread exists
    const thread = await ctx.db.get(args.threadDocId);
    if (!thread) {
      throw new Error(`Thread with ID ${args.threadDocId} not found`);
    }

    // Update the thread with the new message info
    await ctx.db.patch(args.threadDocId, {
      lastMessageId: args.messageId,
      lastMessageText: args.text,
      lastMessageTimestamp: args.timestamp,
      messageCount: thread.messageCount + 1,
      updatedAt: Date.now(),
    });

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
      threadDocId: args.threadDocId, // Link to the specific thread
      createdAt: Date.now(),
    });

    return { success: true, messageId: messageDocId.toString() };
  },
});

// Enhanced mutation to save messages with better thread handling
export const saveMessageWithThreadHandling = mutation({
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

    // If we have a messageThreadId, try to find the existing thread
    if (args.messageThreadId) {
      const existingThread = await ctx.db
        .query("telegram_threads")
        .withIndex("by_chat_and_thread", (q) =>
          q.eq("chatId", args.chatId).eq("threadId", args.messageThreadId!)
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
        // Create new thread for this messageThreadId
        const threadTitle = `Thread ${args.messageThreadId}`;
        threadDocId = await ctx.db.insert("telegram_threads", {
          threadId: args.messageThreadId,
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
    // If we have a userId but no messageThreadId, create/update a user-based thread
    else if (args.userId) {
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
      threadDocId: threadDocId, // Link to the thread
      createdAt: Date.now(),
    });

    return { success: true, messageId: messageDocId.toString() };
  },
});
