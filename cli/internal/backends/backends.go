package backends

import (
	"errors"
	"turbo/internal/api"
	"turbo/internal/backends/nodejs"
	"turbo/internal/fs"
)

var backends = []api.LanguageBackend{
	nodejs.NodejsYarnBackend,
	nodejs.NodejsNpmBackend,
	nodejs.NodejsPnpmBackend,
}

func GetBackend() (*api.LanguageBackend, error) {
	for _, b := range backends {
		if fs.FileExists(b.Specfile) &&
			fs.FileExists(b.Lockfile) {
			return &b, nil
		}
	}

	for _, b := range backends {
		if fs.FileExists(b.Specfile) ||
			fs.FileExists(b.Lockfile) {
			return &b, nil
		}
	}

	return &api.LanguageBackend{}, errors.New("could not determine language / package management backend")
}
