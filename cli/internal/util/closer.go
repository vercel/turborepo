package util

// CloseAndIgnoreError is a utility to tell our linter that we explicitly deem it okay
// to not check a particular error on closing of a resource.
//
// We use `errcheck` as a linter, which is super-opinionated about checking errors,
// even in places where we don't necessarily care to check the error.
//
// `golangci-lint` has a default ignore list for this lint problem (EXC0001) which
// can be used to sidestep this problem but it's possibly a little too-heavy-handed
// in exclusion. At the expense of discoverability, this utility function forces
// opt-in to ignoring errors on closing of things that can be `Close`d.
func CloseAndIgnoreError(closer interface{ Close() error }) {
	_ = closer.Close()
}
