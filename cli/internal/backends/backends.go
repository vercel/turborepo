package backends

import (
	"errors"
	"path/filepath"
	"turbo/internal/api"
	"turbo/internal/backends/nodejs"
	"turbo/internal/fs"
)

var backends = []api.LanguageBackend{
	nodejs.NodejsYarnBackend,
	nodejs.NodejsBerryBackend,
	nodejs.NodejsNpmBackend,
	nodejs.NodejsPnpmBackend,
}

func GetBackend(cwd string) (*api.LanguageBackend, error) {
	return Detect(cwd)
}

func Detect(cwd string) (*api.LanguageBackend, error) {
	possibleBackends := make([]api.LanguageBackend, 0)
	for _, b := range backends {
		if fs.FileExists(filepath.Join(cwd, b.Specfile)) &&
			fs.FileExists(filepath.Join(cwd, b.Lockfile)) {
			possibleBackends = append(possibleBackends, b)
		}
	}

	if len(possibleBackends) == 1 {
		return &possibleBackends[0], nil
	}

	for i, b := range possibleBackends {
		if b.Name == nodejs.NodejsYarnBackend.Name &&
			!fs.PathExists(filepath.Join(cwd, ".yarn/releases")) {
			return &possibleBackends[i], nil
		} else if b.Name == nodejs.NodejsBerryBackend.Name &&
			fs.PathExists(filepath.Join(cwd, ".yarn/releases")) {
			return &possibleBackends[i], nil
		}
	}

	return &api.LanguageBackend{}, errors.New("could not determine language / package management backend")
}
