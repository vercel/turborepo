// Package ci is a simple utility to check if a program is being executed in common CI/CD/PaaS vendors.
// This is a partial port of https://github.com/watson/ci-info
package ci

import "os"

var isCI = os.Getenv("CI") != "" || os.Getenv("CONTINUOUS_INTEGRATION") != "" || os.Getenv("BUILD_NUMBER") != "" || os.Getenv("RUN_ID") != "" || os.Getenv("TEAMCITY_VERSION") != "" || false

// IsCi returns true if the program is executing in a CI/CD environment
func IsCi() bool {
	return isCI
}

// Name returns the name of the CI vendor
func Name() string {
	return Info().Name
}

// Info returns information about a CI vendor
func Info() Vendor {
	for _, env := range Vendors {
		if env.EvalEnv != nil {
			for name, value := range env.EvalEnv {
				if os.Getenv(name) == value {
					return env
				}
			}
		} else {
			if os.Getenv(env.Env) != "" {
				return env
			}
		}
	}
	return Vendor{}
}
