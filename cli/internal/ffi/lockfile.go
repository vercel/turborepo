package ffi

// #include "bindings.h"
//
// #cgo LDFLAGS: -L${SRCDIR} -lturborepo_ffi
// #cgo windows LDFLAGS: -lole32 -lbcrypt -lws2_32 -luserenv
import "C"

import (
	"errors"

	ffi_proto "github.com/vercel/turbo/cli/internal/ffi/proto"
)

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
