package ffi

// ffi
//
// Please read the notes about safety (marked with `SAFETY`) in both this file,
// and in turborepo-ffi/lib.rs before modifying this file.

// #include "bindings.h"
//
// #cgo darwin,arm64 LDFLAGS:  -L${SRCDIR} -lturborepo_ffi_darwin_arm64  -lz -liconv
// #cgo darwin,amd64 LDFLAGS:  -L${SRCDIR} -lturborepo_ffi_darwin_amd64  -lz -liconv
// #cgo linux,arm64,staticbinary LDFLAGS:   -L${SRCDIR} -lturborepo_ffi_linux_arm64 -lunwind
// #cgo linux,amd64,staticbinary LDFLAGS:   -L${SRCDIR} -lturborepo_ffi_linux_amd64 -lunwind
// #cgo linux,arm64,!staticbinary LDFLAGS:   -L${SRCDIR} -lturborepo_ffi_linux_arm64 -lz
// #cgo linux,amd64,!staticbinary LDFLAGS:   -L${SRCDIR} -lturborepo_ffi_linux_amd64 -lz
// #cgo windows,amd64 LDFLAGS: -L${SRCDIR} -lturborepo_ffi_windows_amd64 -lole32 -lbcrypt -lws2_32 -luserenv
import "C"

import (
	"errors"
	"fmt"
	"reflect"
	"unsafe"

	ffi_proto "github.com/vercel/turbo/cli/internal/ffi/proto"
	"google.golang.org/protobuf/proto"
)

// Unmarshal consumes a buffer and parses it into a proto.Message
func Unmarshal[M proto.Message](b C.Buffer, c M) error {
	bytes := toBytes(b)
	if err := proto.Unmarshal(bytes, c); err != nil {
		return err
	}

	// free the buffer on the rust side
	//
	// SAFETY: do not use `C.free_buffer` to free a buffer that has been allocated
	// on the go side. If you happen to accidentally use the wrong one, you can
	// expect a segfault on some platforms. This is the only valid callsite.
	C.free_buffer(b)

	return nil
}

// Marshal consumes a proto.Message and returns a bufferfire
//
// NOTE: the buffer must be freed by calling `Free` on it
func Marshal[M proto.Message](c M) C.Buffer {
	bytes, err := proto.Marshal(c)
	if err != nil {
		panic(err)
	}

	return toBuffer(bytes)
}

// Free frees a buffer that has been allocated *on the go side*.
//
// SAFETY: this is not the same as `C.free_buffer`, which frees a buffer that
// has been allocated *on the rust side*. If you happen to accidentally use
// the wrong one, you can expect a segfault on some platforms.
//
// EXAMPLE: it is recommended use this function via a `defer` statement, like so:
//
//	reqBuf := Marshal(&req)
//	defer reqBuf.Free()
func (c C.Buffer) Free() {
	C.free(unsafe.Pointer(c.data))
}

// rather than use C.GoBytes, we use this function to avoid copying the bytes,
// since it is going to be immediately Unmarshalled into a proto.Message
//
// SAFETY: go slices contain a pointer to an underlying buffer with a length.
// if the buffer is known to the garbage collector, dropping the last slice will
// cause the memory to be freed. this memory is owned by the rust side (and is
// not known the garbage collector), so dropping the slice will do nothing
func toBytes(b C.Buffer) []byte {
	var out []byte

	len := (uint32)(b.len)

	sh := (*reflect.SliceHeader)(unsafe.Pointer(&out))
	sh.Data = uintptr(unsafe.Pointer(b.data))
	sh.Len = int(len)
	sh.Cap = int(len)

	return out
}

func toBuffer(bytes []byte) C.Buffer {
	b := C.Buffer{}
	b.len = C.uint(len(bytes))
	b.data = (*C.uchar)(C.CBytes(bytes))
	return b
}

// GetTurboDataDir returns the path to the Turbo data directory
func GetTurboDataDir() string {
	buffer := C.get_turbo_data_dir()
	resp := ffi_proto.TurboDataDirResp{}
	if err := Unmarshal(buffer, resp.ProtoReflect().Interface()); err != nil {
		panic(err)
	}
	return resp.Dir
}

// Go convention is to use an empty string for an uninitialized or null-valued
// string. Rust convention is to use an Option<String> for the same purpose, which
// is encoded on the Go side as *string. This converts between the two.
func stringToRef(s string) *string {
	if s == "" {
		return nil
	}
	return &s
}

// ChangedFiles returns the files changed in between two commits, the workdir and the index, and optionally untracked files
func ChangedFiles(gitRoot string, turboRoot string, fromCommit string, toCommit string) ([]string, error) {
	fromCommitRef := stringToRef(fromCommit)

	req := ffi_proto.ChangedFilesReq{
		GitRoot:    gitRoot,
		FromCommit: fromCommitRef,
		ToCommit:   toCommit,
		TurboRoot:  turboRoot,
	}

	reqBuf := Marshal(&req)
	defer reqBuf.Free()

	respBuf := C.changed_files(reqBuf)

	resp := ffi_proto.ChangedFilesResp{}
	if err := Unmarshal(respBuf, resp.ProtoReflect().Interface()); err != nil {
		panic(err)
	}
	if err := resp.GetError(); err != "" {
		return nil, errors.New(err)
	}

	return resp.GetFiles().GetFiles(), nil
}

// PreviousContent returns the content of a file at a previous commit
func PreviousContent(gitRoot, fromCommit, filePath string) ([]byte, error) {
	req := ffi_proto.PreviousContentReq{
		GitRoot:    gitRoot,
		FromCommit: fromCommit,
		FilePath:   filePath,
	}

	reqBuf := Marshal(&req)
	defer reqBuf.Free()

	respBuf := C.previous_content(reqBuf)

	resp := ffi_proto.PreviousContentResp{}
	if err := Unmarshal(respBuf, resp.ProtoReflect().Interface()); err != nil {
		panic(err)
	}
	content := resp.GetContent()
	if err := resp.GetError(); err != "" {
		return nil, errors.New(err)
	}

	return []byte(content), nil
}

// TransitiveDeps returns the transitive external deps for all provided workspaces
func TransitiveDeps(content []byte, packageManager string, workspaces map[string]map[string]string, resolutions map[string]string) (map[string]*ffi_proto.LockfilePackageList, error) {
	var additionalData *ffi_proto.AdditionalBerryData
	if resolutions != nil {
		additionalData = &ffi_proto.AdditionalBerryData{Resolutions: resolutions}
	}
	flatWorkspaces := make(map[string]*ffi_proto.PackageDependencyList)
	for workspace, deps := range workspaces {
		packageDependencyList := make([]*ffi_proto.PackageDependency, len(deps))
		i := 0
		for name, version := range deps {
			packageDependencyList[i] = &ffi_proto.PackageDependency{
				Name:  name,
				Range: version,
			}
			i++
		}
		flatWorkspaces[workspace] = &ffi_proto.PackageDependencyList{List: packageDependencyList}
	}
	req := ffi_proto.TransitiveDepsRequest{
		Contents:       content,
		PackageManager: toPackageManager(packageManager),
		Workspaces:     flatWorkspaces,
		Resolutions:    additionalData,
	}
	reqBuf := Marshal(&req)
	resBuf := C.transitive_closure(reqBuf)
	reqBuf.Free()

	resp := ffi_proto.TransitiveDepsResponse{}
	if err := Unmarshal(resBuf, resp.ProtoReflect().Interface()); err != nil {
		panic(err)
	}

	if err := resp.GetError(); err != "" {
		return nil, errors.New(err)
	}

	dependencies := resp.GetDependencies()
	return dependencies.GetDependencies(), nil
}

func toPackageManager(packageManager string) ffi_proto.PackageManager {
	switch packageManager {
	case "npm":
		return ffi_proto.PackageManager_NPM
	case "berry":
		return ffi_proto.PackageManager_BERRY
	default:
		panic(fmt.Sprintf("Invalid package manager string: %s", packageManager))
	}
}

// Subgraph returns the contents of a lockfile subgraph
func Subgraph(packageManager string, content []byte, workspaces []string, packages []string, resolutions map[string]string) ([]byte, error) {
	var additionalData *ffi_proto.AdditionalBerryData
	if resolutions != nil {
		additionalData = &ffi_proto.AdditionalBerryData{Resolutions: resolutions}
	}
	req := ffi_proto.SubgraphRequest{
		Contents:       content,
		Workspaces:     workspaces,
		Packages:       packages,
		PackageManager: toPackageManager(packageManager),
		Resolutions:    additionalData,
	}
	reqBuf := Marshal(&req)
	resBuf := C.subgraph(reqBuf)
	reqBuf.Free()

	resp := ffi_proto.SubgraphResponse{}
	if err := Unmarshal(resBuf, resp.ProtoReflect().Interface()); err != nil {
		panic(err)
	}

	if err := resp.GetError(); err != "" {
		return nil, errors.New(err)
	}

	return resp.GetContents(), nil
}

// Patches returns all patch files referenced in the lockfile
func Patches(content []byte, packageManager string) []string {
	req := ffi_proto.PatchesRequest{
		Contents:       content,
		PackageManager: toPackageManager(packageManager),
	}
	reqBuf := Marshal(&req)
	resBuf := C.patches(reqBuf)
	reqBuf.Free()

	resp := ffi_proto.PatchesResponse{}
	if err := Unmarshal(resBuf, resp.ProtoReflect().Interface()); err != nil {
		panic(err)
	}
	if err := resp.GetError(); err != "" {
		panic(err)
	}

	return resp.GetPatches().GetPatches()
}

// RecursiveCopy copies src and its contents to dst
func RecursiveCopy(src string, dst string) error {
	req := ffi_proto.RecursiveCopyRequest{
		Src: src,
		Dst: dst,
	}
	reqBuf := Marshal(&req)
	resBuf := C.recursive_copy(reqBuf)
	reqBuf.Free()

	resp := ffi_proto.RecursiveCopyResponse{}
	if err := Unmarshal(resBuf, resp.ProtoReflect().Interface()); err != nil {
		panic(err)
	}
	// Error is optional, so a nil value means no error was set
	// GetError() papers over the difference and returns the zero
	// value if it isn't set, so we need to check the value directly.
	if resp.Error != nil {
		return errors.New(*resp.Error)
	}
	return nil
}

// GlobalChange checks if there are any differences between lockfiles that would completely invalidate
// the cache.
func GlobalChange(packageManager string, prevContents []byte, currContents []byte) bool {
	req := ffi_proto.GlobalChangeRequest{
		PackageManager: toPackageManager(packageManager),
		PrevContents:   prevContents,
		CurrContents:   currContents,
	}
	reqBuf := Marshal(&req)
	resBuf := C.patches(reqBuf)
	reqBuf.Free()

	resp := ffi_proto.GlobalChangeResponse{}
	if err := Unmarshal(resBuf, resp.ProtoReflect().Interface()); err != nil {
		panic(err)
	}

	return resp.GetGlobalChange()
}
