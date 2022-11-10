package cmd

//import (
//	"reflect"
//	"testing"
//
//	"github.com/vercel/turbo/cli/internal/cmdutil"
//	"github.com/vercel/turbo/cli/internal/signals"
//)
//
//func TestDefaultCmd(t *testing.T) {
//	testCases := []struct {
//		name         string
//		args         []string
//		defaultAdded bool
//	}{
//		{
//			name:         "normal run build",
//			args:         []string{"run", "build"},
//			defaultAdded: false,
//		},
//		{
//			name:         "empty args",
//			args:         []string{},
//			defaultAdded: true,
//		},
//		{
//			name:         "root help",
//			args:         []string{"--help"},
//			defaultAdded: false,
//		},
//		{
//			name:         "run help",
//			args:         []string{"run", "--help"},
//			defaultAdded: false,
//		},
//		{
//			name:         "version",
//			args:         []string{"--version"},
//			defaultAdded: false,
//		},
//		{
//			name:         "heap",
//			args:         []string{"--heap", "my-heap-profile", "some-task", "--cpuprofile", "my-profile"},
//			defaultAdded: true,
//		},
//	}
//	for _, tc := range testCases {
//		args := tc.args
//		t.Run(tc.name, func(t *testing.T) {
//			signalWatcher := signals.NewWatcher()
//			helper := cmdutil.NewHelper("test-version")
//			root := getCmd(helper, signalWatcher)
//			resolved := resolveArgs(root, args)
//			defaultAdded := !reflect.DeepEqual(args, resolved)
//			if defaultAdded != tc.defaultAdded {
//				t.Errorf("Default command added got %v, want %v", defaultAdded, tc.defaultAdded)
//			}
//		})
//	}
//}
