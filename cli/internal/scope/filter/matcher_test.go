package filter

import "testing"

func TestMatcher(t *testing.T) {
	testCases := map[string][]struct {
		test string
		want bool
	}{
		"*": {
			{
				test: "@eslint/plugin-foo",
				want: true,
			},
			{
				test: "express",
				want: true,
			},
		},
		"eslint-*": {
			{
				test: "eslint-plugin-foo",
				want: true,
			},
			{
				test: "express",
				want: false,
			},
		},
		"*plugin*": {
			{
				test: "@eslint/plugin-foo",
				want: true,
			},
			{
				test: "express",
				want: false,
			},
		},
		"a*c": {
			{
				test: "abc",
				want: true,
			},
		},
		"*-positive": {
			{
				test: "is-positive",
				want: true,
			},
		},
	}
	for pattern, tests := range testCases {
		matcher, err := matcherFromPattern(pattern)
		if err != nil {
			t.Fatalf("failed to compile match pattern %v, %v", pattern, err)
		}
		for _, testCase := range tests {
			got := matcher(testCase.test)
			if got != testCase.want {
				t.Errorf("%v.match(%v) got %v, want %v", pattern, testCase.test, got, testCase.want)
			}
		}
	}
}
