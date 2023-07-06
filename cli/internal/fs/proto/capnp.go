package proto

import (
	"encoding/hex"
	"sort"

	capnp "capnproto.org/go/capnp/v3"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"github.com/vercel/turbo/cli/internal/util"
	"github.com/vercel/turbo/cli/internal/xxhash"
)

// HashTaskHashable performs the hash for a TaskHashable, using capnproto for stable cross platform / language hashing
//
// NOTE: This function is _explicitly_ ordered and should not be sorted.
//
//		Order is important for the hash, and is as follows:
//		- GlobalHash
//		- PackageDir
//		- HashOfFiles
//		- ExternalDepsHash
//		- Task
//		- EnvMode
//		- Outputs
//		- TaskDependencyHashes
//		- PassThruArgs
//		- Env
//		- PassThroughEnv
//	 - DotEnv
//	 - ResolvedEnvVars
func HashTaskHashable(task *fs.TaskHashable) (string, error) {
	arena := capnp.SingleSegment(nil)

	_, seg, err := capnp.NewMessage(arena)
	if err != nil {
		return "", err
	}

	task_msg, err := NewRootTaskHashable(seg)
	if err != nil {
		return "", err
	}

	err = task_msg.SetGlobalHash(task.GlobalHash)
	if err != nil {
		return "", err
	}

	err = task_msg.SetPackageDir(task.PackageDir.ToString())
	if err != nil {
		return "", err
	}

	err = task_msg.SetHashOfFiles(task.HashOfFiles)
	if err != nil {
		return "", err
	}

	err = task_msg.SetExternalDepsHash(task.ExternalDepsHash)
	if err != nil {
		return "", err
	}

	err = task_msg.SetTask(task.Task)
	if err != nil {
		return "", err
	}

	{
		var envMode TaskHashable_EnvMode
		switch task.EnvMode {
		case util.Infer:
			envMode = TaskHashable_EnvMode_infer
		case util.Loose:
			envMode = TaskHashable_EnvMode_loose
		case util.Strict:
			envMode = TaskHashable_EnvMode_strict
		}

		task_msg.SetEnvMode(envMode)
	}

	{
		arena := capnp.SingleSegment(nil)
		_, seg, _ := capnp.NewMessage(arena)
		deps, _ := NewTaskOutputs(seg)

		err = assignList(task.Outputs.Inclusions, deps.SetInclusions, seg)
		if err != nil {
			return "", err
		}

		assignList(task.Outputs.Exclusions, deps.SetExclusions, seg)
		if err != nil {
			return "", err
		}

		task_msg.SetOutputs(deps)
	}

	err = assignList(task.TaskDependencyHashes, task_msg.SetTaskDependencyHashes, seg)
	if err != nil {
		return "", err
	}

	err = assignList(task.PassThruArgs, task_msg.SetPassThruArgs, seg)
	if err != nil {
		return "", err
	}

	err = assignList(task.Env, task_msg.SetEnv, seg)
	if err != nil {
		return "", err
	}

	err = assignList(task.PassThroughEnv, task_msg.SetPassThruEnv, seg)
	if err != nil {
		return "", err
	}

	err = assignAnchoredUnixArray(task.DotEnv, task_msg.SetDotEnv, seg)
	if err != nil {
		return "", err
	}

	err = assignList(task.ResolvedEnvVars, task_msg.SetResolvedEnvVars, seg)
	if err != nil {
		return "", err
	}

	out, err := HashMessage(task_msg.Message())

	return out, nil
}

// HashGlobalHashable performs the hash for a GlobalHashable, using capnproto for stable cross platform / language hashing
//
// NOTE: This function is _explicitly_ ordered and should not be sorted.
//
//		Order is important for the hash, and is as follows:
//		- GlobalCacheKey
//		- GlobalFileHashMap
//		- RootExternalDepsHash
//    - Env
//    - ResolvedEnvVars
//    - PassThroughEnv
//    - EnvMode
//    - FrameworkInference
//    - DotEnv

func HashGlobalHashable(global *fs.GlobalHashable) (string, error) {
	arena := capnp.SingleSegment(nil)

	_, seg, err := capnp.NewMessage(arena)
	if err != nil {
		return "", err
	}

	global_msg, err := NewRootGlobalHashable(seg)
	if err != nil {
		return "", err
	}

	err = global_msg.SetGlobalCacheKey(global.GlobalCacheKey)
	if err != nil {
		return "", err
	}

	{
		entries, err := global_msg.NewGlobalFileHashMap(int32(len(global.GlobalFileHashMap)))
		if err != nil {
			return "", err
		}

		// get a list of key value pairs and then sort them by key
		// to do this we need three lists, one for the keys, one for the string representation of the keys,
		// and one for the indices of the keys
		keys := make([]turbopath.AnchoredUnixPath, len(global.GlobalFileHashMap))
		keyStrs := make([]string, len(global.GlobalFileHashMap))
		keyIndices := make([]int, len(global.GlobalFileHashMap))

		i := 0
		for k := range global.GlobalFileHashMap {
			keys[i] = k
			keyStrs[i] = k.ToString()
			keyIndices[i] = i
			i++
		}

		sort.Slice(keyIndices, func(i, j int) bool {
			return keyStrs[keyIndices[i]] < keyStrs[keyIndices[j]]
		})

		for i, idx := range keyIndices {
			entry := entries.At(i)
			if err != nil {
				return "", err
			}

			err = entry.SetKey(keyStrs[idx])
			if err != nil {
				return "", err
			}

			err = entry.SetValue(global.GlobalFileHashMap[keys[idx]])
			if err != nil {
				return "", err
			}
		}

		if err != nil {
			return "", err
		}
	}

	err = global_msg.SetRootExternalDepsHash(global.RootExternalDepsHash)
	if err != nil {
		return "", err
	}

	err = assignList(global.Env, global_msg.SetEnv, seg)
	if err != nil {
		return "", err
	}

	err = assignList(global.ResolvedEnvVars, global_msg.SetResolvedEnvVars, seg)
	if err != nil {
		return "", err
	}

	err = assignList(global.PassThroughEnv, global_msg.SetPassThroughEnv, seg)
	if err != nil {
		return "", err
	}

	{
		var envMode GlobalHashable_EnvMode
		switch global.EnvMode {
		case util.Infer:
			envMode = GlobalHashable_EnvMode_infer
		case util.Loose:
			envMode = GlobalHashable_EnvMode_loose
		case util.Strict:
			envMode = GlobalHashable_EnvMode_strict
		}

		global_msg.SetEnvMode(envMode)
	}

	global_msg.SetFrameworkInference(global.FrameworkInference)

	err = assignAnchoredUnixArray(global.DotEnv, global_msg.SetDotEnv, seg)
	if err != nil {
		return "", err
	}

	out, err := HashMessage(global_msg.Message())

	return out, nil
}

func HashMessage(msg *capnp.Message) (string, error) {
	bytes, err := msg.Marshal()
	if err != nil {
		return "", err
	}

	digest := xxhash.New()
	digest.Write(bytes)
	out := digest.Sum(nil)

	return hex.EncodeToString(out), nil
}

func assignList(list []string, fn func(capnp.TextList) error, seg *capnp.Segment) error {
	textList, err := capnp.NewTextList(seg, int32(len(list)))
	if err != nil {
		return err
	}
	for i, v := range list {
		textList.Set(i, v)
	}
	return fn(textList)
}

func assignAnchoredUnixArray(paths turbopath.AnchoredUnixPathArray, fn func(capnp.TextList) error, seg *capnp.Segment) error {
	textList, err := capnp.NewTextList(seg, int32(len(paths)))
	if err != nil {
		return err
	}
	for i, v := range paths {
		textList.Set(i, v.ToString())
	}
	return fn(textList)
}
