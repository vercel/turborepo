// fmt.Sprintf with literal arguments, constants, locals and filepath.Join.
package main

import (
	"fmt"
	"os"
	"path/filepath"
)

const localeDir = "locales"

func main() {
	lang := "en"
	p := fmt.Sprintf("%s/%s.json", localeDir, lang)
	a, _ := os.ReadFile(p)
	b, _ := os.ReadFile(filepath.Join("profiles", "release"+".ini"))
	fmt.Println(len(a) + len(b))
}
