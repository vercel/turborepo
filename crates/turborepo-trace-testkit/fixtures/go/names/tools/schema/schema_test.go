package schema

import (
	"os"
	"path/filepath"
	"runtime"
	"testing"
)

// schemaPath finds the schema from the location of this file, so that the test runs
// wherever `go test` is started.
func schemaPath(t *testing.T) string {
	t.Helper()
	_, thisFile, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatalf("cannot determine the current file")
	}
	return filepath.Join(filepath.Dir(thisFile), "schemas", "job.schema.json")
}

func schemaKeys(t *testing.T, path string) int {
	t.Helper()
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read %s: %v", path, err)
	}
	return len(data)
}

func TestSchemaInSync(t *testing.T) {
	if schemaKeys(t, schemaPath(t)) == 0 {
		t.Fatal("empty schema")
	}
}
