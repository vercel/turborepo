package fs

// TurboConfigJSON is the root turborepo configuration
type TurboConfigJSON struct {
	// Base Git branch
	BaseBranch string `json:"baseBranch,omitempty"`
	// Global root filesystem dependencies
	GlobalDependencies []string `json:"globalDependencies,omitempty"`
	// HashInputCommands are an array of commands whose outputs (stdout/stderr)
	// will be included in the global hash value. This value is included in all
	// other package-task hashes
	HashInputCommands []string `json:"hashInputCommands,omitempty"`
	// HashEnvVars are a list of environment variables whose key-values will be
	// included in the global hash. This is useful for when you want hashes to
	// change based on the value of an env var
	HashEnvVars []string `json:"hashEnvVariables,omitempty"`
	// CacheOptions configure how Turbo will cache tasks
	CacheOptions struct {
		// RemoteCacheUrl is the Remote Cache API URL
		RemoteCacheUrl string `json:"remoteCacheUrl,omitempty"`
		// RemoteOnly forces Turbo to only fetch/put artifacts to the remote cache
		// and avoid local caching
		RemoteOnly bool `json:"remoteCacheOnly,omitempty"`
		// LocalCacheDirectory is the relative path to the local filesystem cache
		// directory
		LocalCacheDirectory string `json:"localCacheDirectory,omitempty"`
		// Workers are the number of cache workers in the async cache worker pool
		// use to store task artifacts. Default is runtime.NumCPU() + 2,
		Workers int `json:"cacheWorkers"`
	} `json:"cacheOptions,omitempty"`
	// Pipeline is a map of Turbo pipeline entries which define the task graph
	// and cache behavior on a per task or per package-task basis.
	Pipeline map[string]Pipeline
}

// Pipeline specifies the relationship(s) between package.json
// scripts (i.e. tasks) and caching behavior in a concise manner.
type Pipeline struct {
	// Outputs are an array of globs relative to the package to
	// be cached
	Outputs []string `json:"outputs,omitempty"`
	// Cache is a boolean of whether or not the task should
	// cached
	Cache *bool `json:"cache,omitempty"`
	// DependsOn defines both per-task and topological task dependencies.
	// Topological dependencies are prefixed with a delimiter (^) whereas
	// intra-package dependencies are not.
	DependsOn []string `json:"dependsOn,omitempty"`
}
