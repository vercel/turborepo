export interface TelegramMessage {
  _id: string;
  messageId: number;
  chatId: number;
  userId?: number;
  username?: string;
  firstName?: string;
  lastName?: string;
  text: string;
  messageType: string;
  timestamp: number;
  createdAt: number;
  messageThreadId?: number;
  replyToMessageId?: number;
}

export interface TelegramThread {
  _id: string;
  threadId: number;
  chatId: number;
  title?: string;
  creatorUserId?: number;
  creatorUsername?: string;
  creatorFirstName?: string;
  creatorLastName?: string;
  firstMessageId?: number;
  lastMessageId?: number;
  lastMessageText?: string;
  lastMessageTimestamp?: number;
  messageCount: number;
  isActive: boolean;
  createdAt: number;
  updatedAt: number;
}
