"use client";

import { useState } from "react";
import { useQuery } from "convex/react";
import { api } from "../../convex/_generated/api";
import { Hero } from "../components/ui/hero";
import { Card } from "../components/ui/card";
import { Button } from "../components/ui/button";

interface SendMessageForm {
  chatId: string;
  message: string;
  threadId: string;
}

interface Thread {
  _id: string;
  chatId: number;
  threadId: number;
  title?: string;
  messageCount: number;
}

export default function SendMessagePage() {
  const [form, setForm] = useState<SendMessageForm>({
    chatId: "",
    message: "",
    threadId: "",
  });
  const [isLoading, setIsLoading] = useState(false);
  const [result, setResult] = useState<{
    type: "success" | "error";
    message: string;
  } | null>(null);

  const threads = useQuery(api.threads.getAllActiveThreads, { limit: 100 }) as
    | Thread[]
    | undefined;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();

    if (!form.chatId.trim() || !form.message.trim()) {
      setResult({
        type: "error",
        message: "Please fill in both chat ID and message fields.",
      });
      return;
    }

    setIsLoading(true);
    setResult(null);

    try {
      const chatId = parseInt(form.chatId);
      if (isNaN(chatId)) {
        throw new Error("Chat ID must be a valid number");
      }

      let messageThreadId = undefined;
      if (form.threadId.trim()) {
        const threadIdNum = parseInt(form.threadId);
        if (isNaN(threadIdNum)) {
          throw new Error("Thread ID must be a valid number");
        }
        messageThreadId = threadIdNum;
      }

      // Check if we have a selected thread from the threads list
      const selectedThread = threads?.find(
        (thread: Thread) =>
          thread.chatId === chatId && thread.threadId === messageThreadId
      );

      let response;
      if (selectedThread) {
        // Use thread-specific API if we have a thread document ID
        console.log(
          "Using thread-specific API for thread:",
          selectedThread._id
        );
        response = await fetch("/api/telegram/send-to-thread", {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
          },
          body: JSON.stringify({
            chatId,
            text: form.message,
            threadDocId: selectedThread._id,
            messageThreadId,
          }),
        });
      } else {
        // Use regular API for new threads or non-thread messages
        console.log("Using regular API for message");
        response = await fetch("/api/telegram/send-message", {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
          },
          body: JSON.stringify({
            chatId,
            text: form.message,
            messageThreadId,
          }),
        });
      }

      if (!response.ok) {
        throw new Error("Failed to send message");
      }

      const result = await response.json();

      setResult({
        type: "success",
        message: `Message sent successfully! Message ID: ${
          result.telegramMessageId
        }${messageThreadId ? ` (to thread ${messageThreadId})` : ""}`,
      });

      // Clear the message field but keep the chat ID and thread ID for convenience
      setForm((prev) => ({ ...prev, message: "" }));
    } catch (error) {
      setResult({
        type: "error",
        message:
          error instanceof Error ? error.message : "Failed to send message",
      });
    } finally {
      setIsLoading(false);
    }
  };

  const handleInputChange = (
    e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>
  ) => {
    const { name, value } = e.target;
    setForm((prev) => ({ ...prev, [name]: value }));

    // Clear result when user starts typing
    if (result) {
      setResult(null);
    }
  };

  return (
    <div className="max-w-4xl mx-auto p-6">
      <Hero
        title="Send Telegram Message"
        subtitle="Send a message to any chat through your Telegram bot"
      />

      <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
        <Card className="lg:col-span-2">
          <form onSubmit={handleSubmit} className="space-y-6">
            <div>
              <label
                htmlFor="chatId"
                className="block text-sm font-semibold text-gray-700 dark:text-gray-300 mb-2"
              >
                Chat ID
              </label>
              <input
                type="text"
                id="chatId"
                name="chatId"
                value={form.chatId}
                onChange={handleInputChange}
                placeholder="Enter chat ID (e.g., 123456789)"
                className="w-full px-4 py-3 border-2 border-gray-200 dark:border-gray-600 rounded-xl focus:border-blue-500 focus:outline-none transition-colors bg-white dark:bg-gray-800 text-gray-900 dark:text-white"
                disabled={isLoading}
              />
              <small className="block text-gray-500 dark:text-gray-400 text-sm mt-1">
                You can find the chat ID in the messages page or by sending a
                message to your bot first.
              </small>
            </div>

            <div>
              <label
                htmlFor="threadId"
                className="block text-sm font-semibold text-gray-700 dark:text-gray-300 mb-2"
              >
                Thread ID (Optional)
              </label>
              <input
                type="text"
                id="threadId"
                name="threadId"
                value={form.threadId}
                onChange={handleInputChange}
                placeholder="Enter thread ID (optional, for group threads)"
                className="w-full px-4 py-3 border-2 border-gray-200 dark:border-gray-600 rounded-xl focus:border-blue-500 focus:outline-none transition-colors bg-white dark:bg-gray-800 text-gray-900 dark:text-white"
                disabled={isLoading}
              />
              <small className="block text-gray-500 dark:text-gray-400 text-sm mt-1">
                Leave empty to send to main chat. Use thread ID to send to a
                specific thread in groups.
              </small>
            </div>

            {threads && threads.length > 0 && (
              <div>
                <label className="block text-sm font-semibold text-gray-700 dark:text-gray-300 mb-2">
                  Available Threads
                </label>
                <div className="max-h-48 overflow-y-auto space-y-2 border border-gray-200 dark:border-gray-600 rounded-xl p-3">
                  {threads
                    .filter((thread: Thread) =>
                      form.chatId
                        ? thread.chatId.toString() === form.chatId
                        : true
                    )
                    .slice(0, 5)
                    .map((thread: Thread) => (
                      <div
                        key={thread._id}
                        className="p-3 border border-gray-200 dark:border-gray-600 rounded-lg cursor-pointer hover:border-blue-500 hover:bg-gray-50 dark:hover:bg-gray-700 transition-all"
                        onClick={() => {
                          setForm((prev) => ({
                            ...prev,
                            chatId: thread.chatId.toString(),
                            threadId: thread.threadId.toString(),
                          }));
                        }}
                      >
                        <div className="space-y-1">
                          <div className="font-semibold text-gray-900 dark:text-white">
                            {thread.title || `Thread ${thread.threadId}`}
                          </div>
                          <div className="text-sm text-gray-500 dark:text-gray-400">
                            Chat: {thread.chatId} | Thread: {thread.threadId} |{" "}
                            {thread.messageCount} messages
                          </div>
                        </div>
                      </div>
                    ))}
                </div>
              </div>
            )}

            <div>
              <label
                htmlFor="message"
                className="block text-sm font-semibold text-gray-700 dark:text-gray-300 mb-2"
              >
                Message
              </label>
              <textarea
                id="message"
                name="message"
                value={form.message}
                onChange={handleInputChange}
                placeholder="Type your message here..."
                className="w-full px-4 py-3 border-2 border-gray-200 dark:border-gray-600 rounded-xl focus:border-blue-500 focus:outline-none transition-colors bg-white dark:bg-gray-800 text-gray-900 dark:text-white resize-vertical"
                rows={4}
                disabled={isLoading}
              />
            </div>

            {result && (
              <div
                className={`p-4 rounded-xl font-medium ${
                  result.type === "success"
                    ? "bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200 border border-green-200 dark:border-green-700"
                    : "bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200 border border-red-200 dark:border-red-700"
                }`}
              >
                {result.message}
              </div>
            )}

            <Button
              variant="secondary"
              disabled={
                isLoading || !form.chatId.trim() || !form.message.trim()
              }
              className="w-full"
            >
              {isLoading ? (
                <>
                  <div className="w-4 h-4 border-2 border-white border-t-transparent rounded-full animate-spin"></div>
                  Sending...
                </>
              ) : (
                <>
                  <span>✉️</span>
                  Send Message
                </>
              )}
            </Button>
          </form>
        </Card>

        <Card>
          <h3 className="text-lg font-semibold text-gray-900 dark:text-white mb-4">
            💡 Tips
          </h3>
          <ul className="space-y-3 text-gray-600 dark:text-gray-400">
            <li className="flex items-start gap-2">
              <span className="text-blue-500 mt-1">•</span>
              <span>
                Make sure your bot has permission to send messages to the chat
              </span>
            </li>
            <li className="flex items-start gap-2">
              <span className="text-blue-500 mt-1">•</span>
              <span>For group chats, add your bot to the group first</span>
            </li>
            <li className="flex items-start gap-2">
              <span className="text-blue-500 mt-1">•</span>
              <span>You can find chat IDs in the Messages page</span>
            </li>
            <li className="flex items-start gap-2">
              <span className="text-blue-500 mt-1">•</span>
              <span>
                Test with your own user ID first (send a message to your bot to
                get your chat ID)
              </span>
            </li>
          </ul>
        </Card>
      </div>
    </div>
  );
}
