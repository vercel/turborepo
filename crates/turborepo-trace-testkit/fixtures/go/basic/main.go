// Command basic exercises the Go plugin: imports of every kind, embeds and file reads.
package main

import (
	_ "embed"
	"fmt"
	"os"

	"example.com/basic/internal/cgo"
	"example.com/basic/internal/util"
	"example.com/lib"
	dep "github.com/Upper/dep/sub"
)

//go:embed static/*
var static string

//go:embed tmpl
var templates string

//go:embed missing/*.txt
var missing string

func main() {
	data, _ := os.ReadFile("config.json")
	fmt.Println(string(data), util.Name(), lib.Lib(), dep.Sub(), cgo.Native(), version)
	// os.ReadFile("in-a-comment.json")
	fmt.Println("import \"not/an/import\"")
}
