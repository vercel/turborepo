package fs

import (
	"path/filepath"
	"runtime"
	"testing"

	"github.com/spf13/pflag"
	"github.com/vercel/turborepo/cli/internal/turbopath"
	"gotest.tools/v3/assert"
)

// absolute unix paths parse differently based on whether or not we're on windows.
// Windows will treat them as relative paths, to be made relative to cwd.
// Unix-like OSes will treat them as absolute and can be returned directly.
func platformSpecificAbsoluteUnixPathExpectation(cwd turbopath.AbsolutePath, absoluteUnixPath string) turbopath.AbsolutePath {
	if runtime.GOOS == "windows" {
		return cwd.Join(absoluteUnixPath)
	}
	return UnsafeToAbsolutePath(absoluteUnixPath)
}

func TestAbsPathVar(t *testing.T) {
	cwd, err := GetCwd()
	assert.NilError(t, err, "GetCwd")
	flags := pflag.NewFlagSet("foo", pflag.ContinueOnError)
	var target turbopath.AbsolutePath
	AbsolutePathVar(flags, &target, "foo", cwd, "some usage info", "")

	for _, test := range []struct {
		input    string
		expected turbopath.AbsolutePath
	}{
		{
			"bar",
			cwd.Join("bar"),
		},
		{
			filepath.Join("bar", "baz"),
			cwd.Join("bar", "baz"),
		},
		{
			// explicitly use a unix-like separator, but on a relative path
			"bar/baz",
			cwd.Join("bar", "baz"),
		},
		{
			"/bar/baz",
			platformSpecificAbsoluteUnixPathExpectation(cwd, "/bar/baz"),
		},
	} {
		err = flags.Parse([]string{"--foo", test.input})
		assert.NilError(t, err, "Parse")
		if target != test.expected {
			t.Errorf("path got %v, want %v", target, test.expected)
		}
	}
}
