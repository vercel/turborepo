package routing

import "github.com/acme-test/yaml"

// Route names the destination of a record kind.
func Route(kind string) string {
	var routes map[string]string
	_ = yaml.Unmarshal([]byte("log: sink\n"), &routes)
	return routes[kind]
}
