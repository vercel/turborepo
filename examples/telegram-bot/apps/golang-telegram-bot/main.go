// main.go
package main

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"log"
	"net/http"
	"os"
	"os/signal"
	"time"

	"github.com/go-telegram/bot"
	"github.com/go-telegram/bot/models"
)

// TelegramMessage represents the structure for saving messages to Convex
type TelegramMessage struct {
	MessageID        int    `json:"messageId"`
	ChatID           int64  `json:"chatId"`
	UserID           *int64 `json:"userId,omitempty"`
	Username         string `json:"username,omitempty"`
	FirstName        string `json:"firstName,omitempty"`
	LastName         string `json:"lastName,omitempty"`
	Text             string `json:"text"`
	MessageType      string `json:"messageType"`
	Timestamp        int64  `json:"timestamp"`
	MessageThreadID  *int   `json:"messageThreadId,omitempty"`
	ReplyToMessageID *int   `json:"replyToMessageId,omitempty"`
}

// ConvexResponse represents the response from Convex API
type ConvexResponse struct {
	Success   bool   `json:"success"`
	MessageID string `json:"messageId,omitempty"`
	Message   string `json:"message,omitempty"`
	Error     string `json:"error,omitempty"`
}

var (
	convexURL string
	httpClient *http.Client
)

func main() {
	// Configure logging with timestamps for Docker
	log.SetFlags(log.LstdFlags | log.Lmicroseconds)
	log.SetPrefix("[TELEGRAM-BOT] ")

	// Get bot token from environment variable
	botToken := os.Getenv("TELEGRAM_TOKEN")
	if botToken == "" {
		log.Fatal("❌ TELEGRAM_TOKEN environment variable is required")
	}

	// Get Convex URL from environment variable
	convexURL = os.Getenv("CONVEX_URL")
	if convexURL == "" {
		convexURL = "http://convex-backend:3211" // Default for Docker
	}

	// Initialize HTTP client with timeout
	httpClient = &http.Client{
		Timeout: 10 * time.Second,
	}

	log.Printf("🚀 Initializing Telegram bot at %s", time.Now().Format(time.RFC3339))
	log.Printf("🔗 Convex backend URL: %s", convexURL)

	// Create context that will be cancelled on interrupt signal
	ctx, cancel := signal.NotifyContext(context.Background(), os.Interrupt)
	defer cancel()

	// Configure bot options with default handler
	opts := []bot.Option{
		bot.WithDefaultHandler(messageHandler),
	}

	// Create new bot instance
	b, err := bot.New(botToken, opts...)
	if err != nil {
		log.Fatalf("❌ Failed to create bot: %v", err)
	}

	log.Println("✅ Telegram bot started successfully and listening for messages...")
	
	// Start the bot
	b.Start(ctx)
	
	log.Println("🛑 Telegram bot shutting down gracefully...")
}

// saveMessageToConvex saves a telegram message to the Convex database
func saveMessageToConvex(ctx context.Context, msg *models.Message) error {
	// Prepare the message data
	telegramMsg := TelegramMessage{
		MessageID:   msg.ID,
		ChatID:      msg.Chat.ID,
		Text:        msg.Text,
		MessageType: "text",
		Timestamp:   int64(msg.Date) * 1000, // Convert to milliseconds
	}

	// Add user information if available
	if msg.From != nil {
		telegramMsg.UserID = &msg.From.ID
		telegramMsg.Username = msg.From.Username
		telegramMsg.FirstName = msg.From.FirstName
		telegramMsg.LastName = msg.From.LastName
	}

	// Add thread information if available - THIS IS THE KEY FIX
	if msg.MessageThreadID != 0 {
		threadID := msg.MessageThreadID
		telegramMsg.MessageThreadID = &threadID
		log.Printf("🧵 Message is part of thread ID: %d", threadID)
	} else {
		log.Printf("💬 Message is not part of a thread (regular chat message)")
	}

	// Add reply information if available
	if msg.ReplyToMessage != nil {
		replyID := msg.ReplyToMessage.ID
		telegramMsg.ReplyToMessageID = &replyID
		log.Printf("↩️ Message is replying to message ID: %d", replyID)
	}

	// Convert to JSON
	jsonData, err := json.Marshal(telegramMsg)
	if err != nil {
		return fmt.Errorf("failed to marshal message: %w", err)
	}

	log.Printf("🔄 Sending message to Convex: %s", string(jsonData))

	// Create HTTP request
	apiURL := fmt.Sprintf("%s/api/telegram/messages", convexURL)
	log.Printf("🌐 API URL: %s", apiURL)
	
	req, err := http.NewRequestWithContext(ctx, "POST", apiURL, bytes.NewBuffer(jsonData))
	if err != nil {
		return fmt.Errorf("failed to create request: %w", err)
	}

	req.Header.Set("Content-Type", "application/json")

	// Send the request
	log.Printf("📤 Sending HTTP request to Convex...")
	resp, err := httpClient.Do(req)
	if err != nil {
		return fmt.Errorf("failed to send request: %w", err)
	}
	defer resp.Body.Close()

	log.Printf("📥 Received response: Status=%d, ContentLength=%d", resp.StatusCode, resp.ContentLength)

	// Read response body for debugging
	bodyBytes, err := io.ReadAll(resp.Body)
	if err != nil {
		return fmt.Errorf("failed to read response body: %w", err)
	}

	log.Printf("📄 Response body: %s", string(bodyBytes))

	// Parse the response
	var convexResp ConvexResponse
	if err := json.Unmarshal(bodyBytes, &convexResp); err != nil {
		return fmt.Errorf("failed to decode response: %w (body: %s)", err, string(bodyBytes))
	}

	if !convexResp.Success {
		return fmt.Errorf("convex API error: %s", convexResp.Error)
	}

	log.Printf("💾 Message saved to Convex with ID: %s", convexResp.MessageID)
	return nil
}

// messageHandler handles all incoming messages
func messageHandler(ctx context.Context, b *bot.Bot, update *models.Update) {
	// Check if the update contains a message
	if update.Message == nil {
		return
	}

	// Extract user and chat information for logging
	userInfo := "Unknown"
	if update.Message.From != nil {
		userInfo = fmt.Sprintf("%s (@%s, ID: %d)", 
			update.Message.From.FirstName, 
			update.Message.From.Username, 
			update.Message.From.ID)
	}

	chatInfo := fmt.Sprintf("Chat ID: %d, Type: %s", 
		update.Message.Chat.ID, 
		update.Message.Chat.Type)

	// Enhanced logging to show thread information
	threadInfo := ""
	if update.Message.MessageThreadID != 0 {
		threadInfo = fmt.Sprintf(" | Thread ID: %d", update.Message.MessageThreadID)
	}

	// Log incoming message with detailed information
	log.Printf("📨 Message received from %s | %s%s | Text: '%s'", 
		userInfo, chatInfo, threadInfo, update.Message.Text)

	// Save message to Convex database
	if err := saveMessageToConvex(ctx, update.Message); err != nil {
		log.Printf("⚠️ Failed to save message to Convex: %v", err)
		// Continue processing even if save fails
	}

	// Create response message
	responseText := fmt.Sprintf("Message received and saved: %s", update.Message.Text)

	// Prepare response parameters
	sendParams := &bot.SendMessageParams{
		ChatID: update.Message.Chat.ID,
		Text:   responseText,
	}

	// If the original message was in a thread, reply in the same thread
	if update.Message.MessageThreadID != 0 {
		sendParams.MessageThreadID = update.Message.MessageThreadID
		log.Printf("🧵 Replying in thread ID: %d", update.Message.MessageThreadID)
	}

	// Send response back to the chat
	_, err := b.SendMessage(ctx, sendParams)
	
	if err != nil {
		log.Printf("❌ Failed to send message to %s: %v", userInfo, err)
	} else {
		log.Printf("✅ Response sent to %s | Text: '%s'", userInfo, responseText)
	}
}
