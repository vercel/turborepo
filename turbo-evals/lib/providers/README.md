# Custom Model Provider Integration

This directory contains the custom provider implementation that allows AI labs to integrate their models for testing.

## Configuration

### JSON Configuration File

Create or modify `lib/providers/custom-providers.json` with your provider configurations:

```json
[
  {
    "name": "Lab Model X",
    "modelId": "lab-model-x-v1",
    "baseURL": "https://api.lab.ai/v1",
    "apiKey": "your-api-key-here",
    "headers": {
      "User-Agent": "NextEvals/1.0"
    }
  },
  {
    "name": "Lab Model Y",
    "modelId": "lab-model-y-preview",
    "baseURL": "https://preview-api.lab.ai/v1",
    "apiKey": "your-preview-api-key"
  }
]
```

### Getting Started

1. **Copy the example file**:

   ```bash
   cp lib/providers/custom-providers.example.json lib/providers/custom-providers.json
   ```

2. **Edit the configuration**:
   - Update the `name` field with your model's display name
   - Set the `modelId` to your model's identifier
   - Configure the `baseURL` to point to your API endpoint
   - Add your `apiKey` for authentication
   - Optionally add custom `headers`

3. **Your models will automatically appear** in the evaluation system

### Configuration Fields

- `name`: Display name for the model in the evaluation UI
- `modelId`: The model identifier to send to the provider API
- `baseURL`: Base URL for the provider's API (should be OpenAI-compatible)
- `apiKey`: (Optional) API key for authentication. If not provided, will use `CUSTOM_PROVIDER_API_KEY` env var
- `headers`: (Optional) Additional headers to send with requests

## API Compatibility

The custom provider expects an OpenAI-compatible API with the following endpoints:

### Chat Completions

- **Endpoint**: `POST /chat/completions`
- **Request Format**: OpenAI chat completions format
- **Response Format**: OpenAI chat completions format
- **Streaming**: Supported via `stream: true` parameter

### Required Response Fields

```json
{
  "choices": [{
    "message": {
      "role": "assistant",
      "content": "Response text"
    },
    "finish_reason": "stop"
  }],
  "usage": {
    "prompt_tokens": 100,
    "completion_tokens": 50,
    "total_tokens": 150
  }
}
```
