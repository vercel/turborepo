//go:build !windows

package unixpath

func evalSymlinks(path string) (string, error) {
	return walkSymlinks(path)
}
