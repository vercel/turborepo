//go:build rust
// +build rust

package hashing

import (
	"github.com/vercel/turbo/cli/internal/ffi"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

func getPackageFileHashesFromGitIndex(rootPath turbopath.AbsoluteSystemPath, packagePath turbopath.AnchoredSystemPath) (map[turbopath.AnchoredUnixPath]string, error) {
	rawHashes, err := ffi.GetPackageFileHashesFromGitIndex(rootPath.ToString(), packagePath.ToString())
	if err != nil {
		return nil, err
	}

	hashes := make(map[turbopath.AnchoredUnixPath]string, len(rawHashes))
	for rawPath, hash := range rawHashes {
		hashes[turbopath.AnchoredUnixPathFromUpstream(rawPath)] = hash
	}
	return hashes, nil
}

func getPackageFileHashesFromProcessingGitIgnore(rootPath turbopath.AbsoluteSystemPath, packagePath turbopath.AnchoredSystemPath, inputs []string) (map[turbopath.AnchoredUnixPath]string, error) {
	rawHashes, err := ffi.GetPackageFileHashesFromProcessingGitIgnore(rootPath.ToString(), packagePath.ToString(), inputs)
	if err != nil {
		return nil, err
	}

	hashes := make(map[turbopath.AnchoredUnixPath]string, len(rawHashes))
	for rawPath, hash := range rawHashes {
		hashes[turbopath.AnchoredUnixPathFromUpstream(rawPath)] = hash
	}
	return hashes, nil
}

func getPackageFileHashesFromInputs(rootPath turbopath.AbsoluteSystemPath, packagePath turbopath.AnchoredSystemPath, inputs []string) (map[turbopath.AnchoredUnixPath]string, error) {
	rawHashes, err := ffi.GetPackageFileHashesFromInputs(rootPath.ToString(), packagePath.ToString(), inputs)
	if err != nil {
		return nil, err
	}

	hashes := make(map[turbopath.AnchoredUnixPath]string, len(rawHashes))
	for rawPath, hash := range rawHashes {
		hashes[turbopath.AnchoredUnixPathFromUpstream(rawPath)] = hash
	}
	return hashes, nil
}
