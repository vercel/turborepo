package backends

import (
	"errors"
	"github.com/vercel/turborepo/cli/internal/api"
	"github.com/vercel/turborepo/cli/internal/backends/nodejs"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/util"
)

var backends = []api.LanguageBackend{
	nodejs.NodejsYarnBackend,
	nodejs.NodejsBerryBackend,
	nodejs.NodejsNpmBackend,
	nodejs.NodejsPnpmBackend,
}

func GetBackend(cwd string, pkg *fs.PackageJSON) (*api.LanguageBackend, error) {
	for _, b := range backends {
		hit, err := b.Detect(cwd, pkg, &b)
		if err != nil {
			return nil, err
		}
		if hit {
			return &b, nil
		}
	}

	return nil, errors.New(util.Sprintf("could not determine package manager. Please set the \"packageManager\" property in your root package.json (${UNDERLINE}https://nodejs.org/api/packages.html#packagemanager)${RESET} or run `npx @turbo/codemod add-package-manager` in the root of your monorepo."))
}
