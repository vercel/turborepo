package graph

import (
	"testing"

	"gotest.tools/v3/assert"
)

func Test_CommandsInvokingTurbo(t *testing.T) {
	type testCase struct {
		command string
		match   bool
	}
	testCases := []testCase{
		{
			"turbo run foo",
			true,
		},
		{
			"rm -rf ~/Library/Caches/pnpm && turbo run foo && rm -rf ~/.npm",
			true,
		},
		{
			"FLAG=true turbo run foo",
			true,
		},
		{
			"npx turbo run foo",
			true,
		},
		{
			"echo starting; turbo foo; echo done",
			true,
		},
		// We don't catch this as if people are going to try to invoke the turbo
		// binary directly, they'll always be able to work around us.
		{
			"./node_modules/.bin/turbo foo",
			false,
		},
		{
			"rm -rf ~/Library/Caches/pnpm && rm -rf ~/Library/Caches/turbo && rm -rf ~/.npm && rm -rf ~/.pnpm-store && rm -rf ~/.turbo",
			false,
		},
	}

	for _, tc := range testCases {
		assert.Equal(t, commandLooksLikeTurbo(tc.command), tc.match, tc.command)
	}
}
