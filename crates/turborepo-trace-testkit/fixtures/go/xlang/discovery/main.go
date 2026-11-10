// Directory listings: os.ReadDir over a template directory, and an embedded directory.
package main

import (
	"embed"
	"fmt"
	"os"
	"path/filepath"
)

//go:embed static
var static embed.FS

func main() {
	entries, _ := os.ReadDir("templates")
	for _, e := range entries {
		b, _ := os.ReadFile(filepath.Join("templates", e.Name()))
		fmt.Println(len(b))
	}
	s, _ := static.ReadFile("static/app.css")
	fmt.Println(len(s))
}
