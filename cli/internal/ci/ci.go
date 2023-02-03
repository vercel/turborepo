// Package ci is a simple utility to check if a program is being executed in common CI/CD/PaaS vendors.
// This is a partial port of https://github.com/watson/ci-info
package ci

import "os"

var isCI = os.Getenv("BUILD_ID") != "" || os.Getenv("BUILD_NUMBER") != "" || os.Getenv("CI") != "" || os.Getenv("CI_APP_ID") != "" || os.Getenv("CI_BUILD_ID") != "" || os.Getenv("CI_BUILD_NUMBER") != "" || os.Getenv("CI_NAME") != "" || os.Getenv("CONTINUOUS_INTEGRATION") != "" || os.Getenv("RUN_ID") != "" || os.Getenv("TEAMCITY_VERSION") != "" || false

// IsCi returns true if the program is executing in a CI/CD environment
func IsCi() bool {
	return isCI
}

// Name returns the name of the CI vendor
func Name() string {
	return Info().Name
}

// Constant returns the name of the CI vendor as a constant
func Constant() string {
	return Info().Constant
}

// Info returns information about a CI vendor
func Info() Vendor {
	// check both the env var key and value
	for _, env := range Vendors {
		if env.EvalEnv != nil {
			for name, value := range env.EvalEnv {
				if os.Getenv(name) == value {
					return env
				}
			}
		} else {
			// check for any of the listed env var keys, with any value
			if env.Env.Any != nil && len(env.Env.Any) > 0 {
				for _, envVar := range env.Env.Any {
					if os.Getenv(envVar) != "" {
						return env
					}
				}
				// check for all of the listed env var keys, with any value
			} else if env.Env.All != nil && len(env.Env.All) > 0 {
				all := true
				for _, envVar := range env.Env.All {
					if os.Getenv(envVar) == "" {
						all = false
						break
					}
				}
				if all {
					return env
				}
			}
		}
	}
	return Vendor{}
}
