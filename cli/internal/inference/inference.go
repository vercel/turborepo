package inference

import "github.com/vercel/turbo/cli/internal/fs"

// Framework is an identifier for something that we wish to inference against.
type Framework struct {
	Slug            string
	EnvPrefix       string
	DependencyMatch matcher
}

type matcher struct {
	strategy     matchStrategy
	dependencies []string
}

type matchStrategy int

const (
	all matchStrategy = iota + 1
	some
)

var _frameworks = []Framework{
	{
		Slug:      "blitzjs",
		EnvPrefix: "NEXT_PUBLIC_",
		DependencyMatch: matcher{
			strategy:     all,
			dependencies: []string{"blitz"},
		},
	},
	{
		Slug:      "nextjs",
		EnvPrefix: "NEXT_PUBLIC_",
		DependencyMatch: matcher{
			strategy:     all,
			dependencies: []string{"next"},
		},
	},
	{
		Slug:      "gatsby",
		EnvPrefix: "GATSBY_",
		DependencyMatch: matcher{
			strategy:     all,
			dependencies: []string{"gatsby"},
		},
	},
	{
		Slug:      "astro",
		EnvPrefix: "PUBLIC_",
		DependencyMatch: matcher{
			strategy:     all,
			dependencies: []string{"astro"},
		},
	},
	{
		Slug:      "solidstart",
		EnvPrefix: "VITE_",
		DependencyMatch: matcher{
			strategy:     all,
			dependencies: []string{"solid-js", "solid-start"},
		},
	},
	{
		Slug:      "vue",
		EnvPrefix: "VUE_APP_",
		DependencyMatch: matcher{
			strategy:     all,
			dependencies: []string{"@vue/cli-service"},
		},
	},
	{
		Slug:      "sveltekit",
		EnvPrefix: "VITE_",
		DependencyMatch: matcher{
			strategy:     all,
			dependencies: []string{"@sveltejs/kit"},
		},
	},
	{
		Slug:      "create-react-app",
		EnvPrefix: "REACT_APP_",
		DependencyMatch: matcher{
			strategy:     some,
			dependencies: []string{"react-scripts", "react-dev-utils"},
		},
	},
	{
		Slug:      "nuxtjs",
		EnvPrefix: "NUXT_ENV_",
		DependencyMatch: matcher{
			strategy:     some,
			dependencies: []string{"nuxt", "nuxt-edge", "nuxt3", "nuxt3-edge"},
		},
	},
	{
		Slug:      "redwoodjs",
		EnvPrefix: "REDWOOD_ENV_",
		DependencyMatch: matcher{
			strategy:     all,
			dependencies: []string{"@redwoodjs/core"},
		},
	},
	{
		Slug:      "vite",
		EnvPrefix: "VITE_",
		DependencyMatch: matcher{
			strategy:     all,
			dependencies: []string{"vite"},
		},
	},
	{
		Slug:      "sanity",
		EnvPrefix: "SANITY_STUDIO_",
		DependencyMatch: matcher{
			strategy:     all,
			dependencies: []string{"@sanity/cli"},
		},
	},
}

func (m matcher) match(pkg *fs.PackageJSON) bool {
	deps := pkg.UnresolvedExternalDeps
	// only check dependencies if we're in a non-monorepo
	if pkg.Workspaces != nil && len(pkg.Workspaces) == 0 {
		deps = pkg.Dependencies
	}

	if m.strategy == all {
		for _, dependency := range m.dependencies {
			_, exists := deps[dependency]
			if !exists {
				return false
			}
		}
		return true
	}

	// m.strategy == some
	for _, dependency := range m.dependencies {
		_, exists := deps[dependency]
		if exists {
			return true
		}
	}
	return false
}

func (f Framework) match(pkg *fs.PackageJSON) bool {
	return f.DependencyMatch.match(pkg)
}

// InferFramework returns a reference to a matched framework
func InferFramework(pkg *fs.PackageJSON) *Framework {
	if pkg == nil {
		return nil
	}

	for _, candidateFramework := range _frameworks {
		if candidateFramework.match(pkg) {
			return &candidateFramework
		}
	}

	return nil
}
