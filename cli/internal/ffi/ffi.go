package ffi

// #include "bindings.h"
//
// #cgo darwin LDFLAGS: -L${SRCDIR} -lturborepo_ffi -lz -liconv
// #cgo linux LDFLAGS: -L${SRCDIR} -lturborepo_ffi -lz
// #cgo windows LDFLAGS: -L${SRCDIR} -lturborepo_ffi -lole32 -lbcrypt -lws2_32 -luserenv
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

	b.Free()

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

func (c C.Buffer) Free() {
	C.free(unsafe.Pointer(c.data))
}

// rather than use C.GoBytes, we use this function to avoid copying the bytes,
// since it is going to be immediately Unmarshalled into a proto.Message
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
func ChangedFiles(repoRoot string, fromCommit string, toCommit string, includeUntracked bool, relativeTo string) ([]string, error) {
	fromCommitRef := stringToRef(fromCommit)
	toCommitRef := stringToRef(toCommit)
	relativeToRef := stringToRef(relativeTo)

	req := ffi_proto.ChangedFilesReq{
		RepoRoot:         repoRoot,
		FromCommit:       fromCommitRef,
		ToCommit:         toCommitRef,
		IncludeUntracked: includeUntracked,
		RelativeTo:       relativeToRef,
	}
	reqBuf := Marshal(&req)

	respBuf := C.changed_files(reqBuf)
	reqBuf.Free()

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
func PreviousContent(repoRoot, fromCommit, filePath string) ([]byte, error) {
	req := ffi_proto.PreviousContentReq{
		RepoRoot:   repoRoot,
		FromCommit: fromCommit,
		FilePath:   filePath,
	}

	reqBuf := Marshal(&req)
	respBuf := C.previous_content(reqBuf)
	reqBuf.Free()

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
