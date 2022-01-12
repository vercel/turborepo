package backends

import (
	"errors"
	"turbo/internal/api"
	"turbo/internal/backends/nodejs"
)

var backends = []api.LanguageBackend{
	nodejs.NodejsYarnBackend,
	nodejs.NodejsBerryBackend,
	nodejs.NodejsNpmBackend,
	nodejs.NodejsPnpmBackend,
}

func GetBackend(cwd string) (*api.LanguageBackend, error) {
	for _, b := range backends {
		hit, _ := b.Detect(cwd, &b)
		if hit {
			return &b, nil
		}
	}

	return nil, errors.New("could not determine language / package management backend")
}
