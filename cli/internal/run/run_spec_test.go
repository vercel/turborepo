package run

import (
	"testing"

	"github.com/vercel/turbo/cli/internal/scope"
	"github.com/vercel/turbo/cli/internal/util"
)

func TestSynthesizeCommand(t *testing.T) {
	testCases := []struct {
		filterPatterns  []string
		legacyFilter    scope.LegacyFilter
		passThroughArgs []string
		parallel        bool
		continueOnError bool
		dryRun          bool
		dryRunJSON      bool
		tasks           []string
		expected        string
	}{
		{
			filterPatterns: []string{"my-app"},
			tasks:          []string{"build"},
			expected:       "turbo run build --filter=my-app",
		},
		{
			filterPatterns:  []string{"my-app"},
			tasks:           []string{"build"},
			passThroughArgs: []string{"-v", "--foo=bar"},
			expected:        "turbo run build --filter=my-app -- -v --foo=bar",
		},
		{
			legacyFilter: scope.LegacyFilter{
				Entrypoints:    []string{"my-app"},
				SkipDependents: true,
			},
			tasks:           []string{"build"},
			passThroughArgs: []string{"-v", "--foo=bar"},
			expected:        "turbo run build --filter=my-app -- -v --foo=bar",
		},
		{
			legacyFilter: scope.LegacyFilter{
				Entrypoints:    []string{"my-app"},
				SkipDependents: true,
			},
			filterPatterns:  []string{"other-app"},
			tasks:           []string{"build"},
			passThroughArgs: []string{"-v", "--foo=bar"},
			expected:        "turbo run build --filter=other-app --filter=my-app -- -v --foo=bar",
		},
		{
			legacyFilter: scope.LegacyFilter{
				Entrypoints:         []string{"my-app"},
				IncludeDependencies: true,
				Since:               "some-ref",
			},
			filterPatterns: []string{"other-app"},
			tasks:          []string{"build"},
			expected:       "turbo run build --filter=other-app --filter=...my-app...[some-ref]...",
		},
		{
			filterPatterns:  []string{"my-app"},
			tasks:           []string{"build"},
			parallel:        true,
			continueOnError: true,
			expected:        "turbo run build --filter=my-app --parallel --continue",
		},
		{
			filterPatterns: []string{"my-app"},
			tasks:          []string{"build"},
			dryRun:         true,
			expected:       "turbo run build --filter=my-app --dry",
		},
		{
			filterPatterns: []string{"my-app"},
			tasks:          []string{"build"},
			dryRun:         true,
			dryRunJSON:     true,
			expected:       "turbo run build --filter=my-app --dry=json",
		},
	}

	for _, testCase := range testCases {
		testCase := testCase
		t.Run(testCase.expected, func(t *testing.T) {
			o := Opts{
				scopeOpts: scope.Opts{
					FilterPatterns: testCase.filterPatterns,
					LegacyFilter:   testCase.legacyFilter,
				},
				runOpts: util.RunOpts{
					PassThroughArgs: testCase.passThroughArgs,
					Parallel:        testCase.parallel,
					ContinueOnError: testCase.continueOnError,
					DryRun:          testCase.dryRun,
					DryRunJSON:      testCase.dryRunJSON,
				},
			}
			cmd := o.SynthesizeCommand(testCase.tasks)
			if cmd != testCase.expected {
				t.Errorf("SynthesizeCommand() got %v, want %v", cmd, testCase.expected)
			}
		})
	}

}
