//go:build rust
// +build rust

package hashing

import (
	"github.com/vercel/turbo/cli/internal/ffi"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

func GetPackageFileHashes(rootPath turbopath.AbsoluteSystemPath, packagePath turbopath.AnchoredSystemPath, inputs []string) (map[turbopath.AnchoredUnixPath]string, error) {
	rawHashes, err := ffi.GetPackageFileHashes(rootPath.ToString(), packagePath.ToString(), inputs)
	if err != nil {
		return nil, err
	}

	hashes := make(map[turbopath.AnchoredUnixPath]string, len(rawHashes))
	for rawPath, hash := range rawHashes {
		hashes[turbopath.AnchoredUnixPathFromUpstream(rawPath)] = hash
	}
	return hashes, nil
}

func GetHashesForFiles(rootPath turbopath.AbsoluteSystemPath, files []turbopath.AnchoredSystemPath) (map[turbopath.AnchoredUnixPath]string, error) {
	rawFiles := make([]string, len(files))
	for i, file := range files {
		rawFiles[i] = file.ToString()
	}
	rawHashes, err := ffi.GetHashesForFiles(rootPath.ToString(), rawFiles, false)
	if err != nil {
		return nil, err
	}

	hashes := make(map[turbopath.AnchoredUnixPath]string, len(rawHashes))
	for rawPath, hash := range rawHashes {
		hashes[turbopath.AnchoredUnixPathFromUpstream(rawPath)] = hash
	}
	return hashes, nil
}

func GetHashesForExistingFiles(rootPath turbopath.AbsoluteSystemPath, files []turbopath.AnchoredSystemPath) (map[turbopath.AnchoredUnixPath]string, error) {
	rawFiles := make([]string, len(files))
	for i, file := range files {
		rawFiles[i] = file.ToString()
	}
	rawHashes, err := ffi.GetHashesForFiles(rootPath.ToString(), rawFiles, true)
	if err != nil {
		return nil, err
	}

	hashes := make(map[turbopath.AnchoredUnixPath]string, len(rawHashes))
	for rawPath, hash := range rawHashes {
		hashes[turbopath.AnchoredUnixPathFromUpstream(rawPath)] = hash
	}
	return hashes, nil
}
