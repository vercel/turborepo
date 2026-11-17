FROM golang:1.22 AS build
WORKDIR /src
COPY go.mod go.sum ./
COPY cmd ./cmd
COPY internal ./internal
RUN go build -o /out/server ./cmd/server

FROM gcr.io/distroless/base
COPY --from=build /out/server /server
ENTRYPOINT ["/server"]
