// Package workspace contains some utilities around managing workspaces
package workspace

import "github.com/vercel/turbo/cli/internal/fs"

// Catalog holds information about each workspace in the monorepo.
type Catalog struct {
	PackageJSONs map[string]*fs.PackageJSON
	TurboConfigs map[string]*fs.TurboJSON
}
