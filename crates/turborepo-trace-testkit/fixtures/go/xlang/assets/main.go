// Paths anchored at the source file through runtime.Caller, stored in variables.
package main

import (
	"fmt"
	"os"
	"path/filepath"
	"runtime"
)

func main() {
	_, file, _, _ := runtime.Caller(0)
	dir := filepath.Dir(file)
	config := filepath.Join(dir, "data", "config.json")
	schema := filepath.Join(dir, "data", "schema.sql")
	fmt.Println(size(config) + size(schema))
}

func size(p string) int {
	b, _ := os.ReadFile(p)
	return len(b)
}
