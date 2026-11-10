package main

import "example.com/server/internal/api"

var version = "dev"

func main() { api.Serve(version) }
