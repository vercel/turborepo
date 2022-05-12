package workspace

import (
	"fmt"
	"path/filepath"
	"strings"

	"github.com/bmatcuk/doublestar/v4"
	gitignore "github.com/sabhiram/go-gitignore"
	"github.com/vercel/turborepo/cli/internal/fs"
)

type Workspace struct {
	RootPath fs.AbsolutePath
	Path     string
	SpecFile *fs.PackageJSON
}

func (w *Workspace) Hash() (string, error) {
	// The user wants to hash this Workspace.
	// We account for:
	// - The list of files.
	// - The sub-portion of dependencies that apply to it.

	hashObject, pkgDepsErr := fs.GetPackageDeps(w.RootPath, &fs.PackageDepsOptions{
		PackagePath:   w.SpecFile.Dir,
		InputPatterns: []string{},
	})
	if pkgDepsErr != nil {
		manualHashObject, err := manuallyHashPackage(w.SpecFile, []string{}, w.RootPath)
		if err != nil {
			return "", err
		}
		hashObject = manualHashObject
	}
	hashOfFiles, otherErr := fs.HashObject(hashObject)
	if otherErr != nil {
		return "", otherErr
	}
	return hashOfFiles, nil
}

func (w *Workspace) HashSelective(patterns []string) (string, error) {
	// The user wants to hash this Workspace.
	// We account for:
	// - The list of files specified by the patterns.
	// - The sub-portion of dependencies that apply to it.
	hashObject, pkgDepsErr := fs.GetPackageDeps(w.RootPath, &fs.PackageDepsOptions{
		PackagePath:   w.SpecFile.Dir,
		InputPatterns: patterns,
	})
	if pkgDepsErr != nil {
		manualHashObject, err := manuallyHashPackage(w.SpecFile, patterns, w.RootPath)
		if err != nil {
			return "", err
		}
		hashObject = manualHashObject
	}
	hashOfFiles, otherErr := fs.HashObject(hashObject)
	if otherErr != nil {
		return "", otherErr
	}
	return hashOfFiles, nil
}

func manuallyHashPackage(pkg *fs.PackageJSON, inputs []string, rootPath fs.AbsolutePath) (map[string]string, error) {
	hashObject := make(map[string]string)
	// Instead of implementing all gitignore properly, we hack it. We only respect .gitignore in the root and in
	// the directory of a package.
	ignore, err := safeCompileIgnoreFile(rootPath.Join(".gitignore").ToString())
	if err != nil {
		return nil, err
	}

	ignorePkg, err := safeCompileIgnoreFile(rootPath.Join(pkg.Dir, ".gitignore").ToString())
	if err != nil {
		return nil, err
	}

	includePattern := ""
	if len(inputs) > 0 {
		includePattern = "{" + strings.Join(inputs, ",") + "}"
	}

	pathPrefix := rootPath.Join(pkg.Dir).ToString()
	toTrim := filepath.FromSlash(pathPrefix + "/")
	fs.Walk(pathPrefix, func(name string, isDir bool) error {
		rootMatch := ignore.MatchesPath(name)
		otherMatch := ignorePkg.MatchesPath(name)
		if !rootMatch && !otherMatch {
			if !isDir {
				if includePattern != "" {
					val, err := doublestar.PathMatch(includePattern, name)
					if err != nil {
						return err
					}
					if !val {
						return nil
					}
				}
				hash, err := fs.GitLikeHashFile(name)
				if err != nil {
					return fmt.Errorf("could not hash file %v. \n%w", name, err)
				}
				hashObject[strings.TrimPrefix(name, toTrim)] = hash
			}
		}
		return nil
	})
	return hashObject, nil
}

func safeCompileIgnoreFile(filepath string) (*gitignore.GitIgnore, error) {
	if fs.FileExists(filepath) {
		return gitignore.CompileIgnoreFile(filepath)
	}
	// no op
	return gitignore.CompileIgnoreLines([]string{}...), nil
}
