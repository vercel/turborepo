# Stage 1: Build the Go application
# Using a specific Go version is good for reproducibility
FROM golang:1.24-alpine AS builder

# Set the working directory inside the container
WORKDIR /app

# Copy the Go module files.
# The wildcard in `go.sum*` is the key fix! It copies go.sum if it exists,
# but doesn't fail if it doesn't.
COPY go.mod go.sum* ./

# Download dependencies. Go will create a go.sum file if it's missing.
RUN go mod download

# Copy the rest of your source code
COPY . .

# Build the Go application into a single binary
RUN CGO_ENABLED=0 GOOS=linux go build -o /app/main .

# ---

# Stage 2: Create the final, lightweight production image
FROM alpine:latest

# We'll run the app as a non-root user for better security
RUN addgroup -S appgroup && adduser -S appuser -G appgroup
USER appuser

WORKDIR /home/appuser

# Copy the built binary from the 'builder' stage
COPY --from=builder /app/main .

# The command to run the application
CMD ["./main"]
