"use client";

import { FaceHappy, FaceSad, FaceSmile, FaceUnhappy } from "./faces";
import { useState } from "react";
import * as Popover from "@radix-ui/react-popover";
import { Check } from "./check";
import { Textarea } from "./textarea";
import { Button } from "./button";
import { cn } from "./cn";

export function FeedbackWidget() {
  const [selectedEmoji, setSelectedEmoji] = useState<string | null>(null);
  const [feedback, setFeedback] = useState("");
  const [loading, setLoading] = useState(false);
  const [isSubmitted, setIsSubmitted] = useState(false);
  const [isOpen, setIsOpen] = useState(false);

  const emojis = [
    { emoji: "🤩", component: <FaceSmile />, label: "Love it" },
    { emoji: "🙂", component: <FaceHappy />, label: "Like it" },
    { emoji: "😕", component: <FaceUnhappy />, label: "Dislike it" },
    { emoji: "😭", component: <FaceSad />, label: "Hate it" },
  ];

  const handleSubmit = (e?: React.FormEvent<HTMLFormElement>): void => {
    e?.preventDefault();

    setLoading(true);

    fetch("/api/feedback", {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
      },
      body: JSON.stringify({
        url:
          window.location.hostname === "localhost"
            ? `https://turbo.build/dev-mode${window.location.pathname}`
            : window.location.toString(),
        note: feedback,
        emotion: selectedEmoji,
        label: "turbo-site",
        ua: `turbo-site ${process.env.NEXT_PUBLIC_VERCEL_ENV || ""} + ${
          navigator.userAgent
        } (${navigator.language || "unknown language"})`,
      }),
    })
      .then(() => {
        setFeedback("");
        setIsSubmitted(true);
      })
      .finally(() => {
        setLoading(false);
      });
  };

  // Handle popover state changes and reset component state when closed
  const handleOpenChange = (open: boolean) => {
    setIsOpen(open);

    // If the popover is closing, reset all state values
    if (!open) {
      setSelectedEmoji(null);
      setFeedback("");
      setLoading(false);
      setIsSubmitted(false);
    }
  };

  return (
    <Popover.Root open={isOpen} onOpenChange={handleOpenChange}>
      <Popover.Trigger asChild>
        <Button
          type="button"
          className="inline-flex items-center justify-center w-full bg-white text-black dark:bg-black dark:text-white border border-black/20 dark:border-white/20 hover:bg-gray-100 dark:hover:bg-gray-900 focus:outline-none focus-visible:ring-2 focus-visible:ring-white focus-visible:ring-opacity-75"
          aria-label="Open feedback form"
        >
          Feedback
        </Button>
      </Popover.Trigger>

      <Popover.Portal>
        <Popover.Content
          className="w-[400px] max-xl:hidden animate-in fade-in-0 zoom-in-95 data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=closed]:zoom-out-95"
          sideOffset={5}
          align="end"
        >
          <div className="bg-white dark:bg-black rounded-xl p-4 text-white border border-gray-800 shadow-lg">
            {isSubmitted ? (
              <div className="py-12 flex flex-col items-center justify-center">
                <div className="bg-green-500 rounded-full p-3 mb-6">
                  <Check className="h-6 w-6" />
                </div>
                <p className="text-xl font-medium mb-2 text-black dark:text-white">
                  Your feedback has been received!
                </p>
                <p className="text-gray-400">Thank you for your help.</p>
              </div>
            ) : (
              <form>
                <div className="mb-4">
                  <Textarea
                    placeholder="Your feedback..."
                    className="min-h-28 text-black dark:bg-black border-gray-700 dark:text-white placeholder:text-gray-500 resize-none"
                    value={feedback}
                    onChange={(e) => setFeedback(e.target.value)}
                  />
                </div>

                <div className="flex items-center justify-between text-black dark:text-gray-500">
                  <div className="flex space-x-4">
                    {emojis.map((item) => (
                      <button
                        type="button"
                        key={item.label}
                        onClick={() => setSelectedEmoji(item.emoji)}
                        className={cn(
                          "text-2xl w-7 h-7 p-1 transition-transform hover:scale-110",
                          selectedEmoji === item.emoji
                            ? "bg-blue-400/40 rounded-full"
                            : ""
                        )}
                        aria-label={item.label}
                      >
                        <span className="relative">{item.component}</span>
                      </button>
                    ))}
                  </div>

                  <div className="flex items-center gap-2">
                    <Button
                      type="submit"
                      className="border dark:text-white hover:bg-black/10 dark:hover:bg-white/10"
                      onClick={(e: any) => handleSubmit(e)}
                      disabled={loading || !feedback || !selectedEmoji}
                    >
                      Send
                    </Button>
                  </div>
                </div>
              </form>
            )}
          </div>
          <Popover.Arrow className="fill-gray-800" />
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}
