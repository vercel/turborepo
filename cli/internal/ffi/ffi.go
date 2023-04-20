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
	toCommitRef := stringToRef(toCommit)

	req := ffi_proto.ChangedFilesReq{
		GitRoot:    gitRoot,
		FromCommit: fromCommitRef,
		ToCommit:   toCommitRef,
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

// NpmTransitiveDeps returns the transitive external deps of a given package based on the deps and specifiers given
func NpmTransitiveDeps(content []byte, pkgDir string, unresolvedDeps map[string]string) ([]*ffi_proto.LockfilePackage, error) {
	return transitiveDeps(npmTransitiveDeps, content, pkgDir, unresolvedDeps)
}

func npmTransitiveDeps(buf C.Buffer) C.Buffer {
	return C.npm_transitive_closure(buf)
}

func transitiveDeps(cFunc func(C.Buffer) C.Buffer, content []byte, pkgDir string, unresolvedDeps map[string]string) ([]*ffi_proto.LockfilePackage, error) {
	req := ffi_proto.TransitiveDepsRequest{
		Contents:       content,
		WorkspaceDir:   pkgDir,
		UnresolvedDeps: unresolvedDeps,
	}
	reqBuf := Marshal(&req)
	resBuf := cFunc(reqBuf)
	reqBuf.Free()

	resp := ffi_proto.TransitiveDepsResponse{}
	if err := Unmarshal(resBuf, resp.ProtoReflect().Interface()); err != nil {
		panic(err)
	}

	if err := resp.GetError(); err != "" {
		return nil, errors.New(err)
	}

	list := resp.GetPackages()
	return list.GetList(), nil
}

// NpmSubgraph returns the contents of a npm lockfile subgraph
func NpmSubgraph(content []byte, workspaces []string, packages []string) ([]byte, error) {
	req := ffi_proto.SubgraphRequest{
		Contents:   content,
		Workspaces: workspaces,
		Packages:   packages,
	}
	reqBuf := Marshal(&req)
	resBuf := C.npm_subgraph(reqBuf)
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
