package inference

import (
	"reflect"
	"testing"

	"github.com/vercel/turbo/cli/internal/fs"
)

func getFrameworkBySlug(slug string) *Framework {
	for _, framework := range _frameworks {
		if framework.Slug == slug {
			return &framework
		}
	}
	panic("that framework doesn't exist")
}

func TestInferFramework(t *testing.T) {
	tests := []struct {
		name string
		pkg  *fs.PackageJSON
		want *Framework
	}{
		{
			name: "Hello world",
			pkg:  nil,
			want: nil,
		},
		{
			name: "Empty dependencies",
			pkg:  &fs.PackageJSON{UnresolvedExternalDeps: map[string]string{}},
			want: nil,
		},
		{
			name: "Finds Blitz",
			pkg: &fs.PackageJSON{UnresolvedExternalDeps: map[string]string{
				"blitz": "*",
			}},
			want: getFrameworkBySlug("blitzjs"),
		},
		{
			name: "Order is preserved (returns blitz, not next)",
			pkg: &fs.PackageJSON{UnresolvedExternalDeps: map[string]string{
				"blitz": "*",
				"next":  "*",
			}},
			want: getFrameworkBySlug("blitzjs"),
		},
		{
			name: "Finds next without blitz",
			pkg: &fs.PackageJSON{UnresolvedExternalDeps: map[string]string{
				"next": "*",
			}},
			want: getFrameworkBySlug("nextjs"),
		},
		{
			name: "match strategy of all works (solid)",
			pkg: &fs.PackageJSON{UnresolvedExternalDeps: map[string]string{
				"solid-js":    "*",
				"solid-start": "*",
			}},
			want: getFrameworkBySlug("solidstart"),
		},
		{
			name: "match strategy of some works (nuxt)",
			pkg: &fs.PackageJSON{UnresolvedExternalDeps: map[string]string{
				"nuxt3": "*",
			}},
			want: getFrameworkBySlug("nuxtjs"),
		},
		{
			name: "match strategy of some works (c-r-a)",
			pkg: &fs.PackageJSON{UnresolvedExternalDeps: map[string]string{
				"react-scripts": "*",
			}},
			want: getFrameworkBySlug("create-react-app"),
		},
		{
			name: "Finds next in non monorepo",
			pkg: &fs.PackageJSON{
				Dependencies: map[string]string{
					"next": "*",
				},
				Workspaces: []string{},
			},
			want: getFrameworkBySlug("nextjs"),
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := InferFramework(tt.pkg); !reflect.DeepEqual(got, tt.want) {
				t.Errorf("InferFramework() = %v, want %v", got, tt.want)
			}
		})
	}
}
