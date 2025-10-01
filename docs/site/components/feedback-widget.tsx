"use client";

import { useState } from "react";
import * as Popover from "@radix-ui/react-popover";
import { FaceHappy, FaceSad, FaceSmile, FaceUnhappy } from "./faces";
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

  const handleSubmit = (e?: React.MouseEvent<HTMLButtonElement>): void => {
    e?.preventDefault();

    setLoading(true);

    void fetch("/api/feedback", {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
      },
      body: JSON.stringify({
        url:
          window.location.hostname === "localhost"
            ? `https://turborepo.com/dev-mode${window.location.pathname}`
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
          variant="outline"
          size="sm"
          aria-label="Open feedback form"
        >
          Feedback
        </Button>
      </Popover.Trigger>

      <Popover.Portal>
        <Popover.Content
          className="w-[400px] max-md:hidden z-50 animate-in fade-in-0 zoom-in-95 data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=closed]:zoom-out-95"
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
                <p className="text-gray-900">Thank you for your help.</p>
              </div>
            ) : (
              <form>
                <div className="mb-4">
                  <Textarea
                    placeholder="Your feedback..."
                    className="min-h-28 text-black dark:bg-black border-gray-700 dark:text-white placeholder:text-gray-500 resize-none"
                    value={feedback}
                    onChange={(e) => {
                      setFeedback(e.target.value);
                    }}
                  />
                </div>

                {!selectedEmoji && feedback ? (
                  <p
                    className="text-red-900 text-right mb-4 text-sm"
                    role="alert"
                    aria-live="assertive"
                    id="emoji-selection-error"
                  >
                    Please select an emoji.
                  </p>
                ) : (
                  <div className="h-9" />
                )}

                <div className="flex items-center justify-between text-black dark:text-gray-900">
                  <div className="flex space-x-4">
                    {emojis.map((item) => {
                      return (
                        <button
                          type="button"
                          key={item.label}
                          onClick={() => {
                            setSelectedEmoji(item.emoji);
                          }}
                          className={cn(
                            "text-2xl w-7 h-7 p-1 transition-transform hover:scale-110",
                            selectedEmoji === item.emoji
                              ? "bg-blue-400 rounded-full"
                              : ""
                          )}
                          aria-label={item.label}
                          aria-describedby={
                            !selectedEmoji && feedback
                              ? "emoji-selection-error"
                              : undefined
                          }
                        >
                          <span className="relative">{item.component}</span>
                        </button>
                      );
                    })}
                  </div>

                  <div className="flex items-center gap-2">
                    <Button
                      type="submit"
                      onClick={(e: React.MouseEvent<HTMLButtonElement>) => {
                        handleSubmit(e);
                      }}
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
