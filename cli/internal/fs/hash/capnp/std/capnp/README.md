This directory contains go packages for all of the capnproto
schemas that ship with capnproto itself. They are generated with
the help of `./gen.sh`. Though some manual modifications have been made
to the schema to correct name collisions (See below).

# `./gen.sh` Usage

Executing:

    ./gen.sh import /path/to/schemas

Will copy all the `*.capnp` files from the given directory into this
directory, and add annotations to them specifying package names and
import paths for `capnpc-go`.

    ./gen.sh compile

Will generate go packages for each of the schemas in the current
directory.

    ./gen.sh clean-go

Will remove all generated go source files from this directory. Finally,

    ./gen.sh clean-all

Will remove both the go source files and the imported schemas.

`gen.sh` does some name mangling to ensure that the generated packages
are actually legal go. However, this is not meant to be a
general-purpose solution; it is only intended to work for the base
schemas.

# Extra Annotations

Under certain circumstances, `capnpc-go` will sometimes generate illegal
go code. As an example, if two declarations `Foo` and `foo` exist in the
schema, `capnpc-go` will capitalize the latter so that it will be
exported, which will cause a name collision.

To address this kind of issue, some of the schema have been manually
modified after importing, adding `$Go.name` annotations which prevent
these errors.

# Versions

The schemas checked in to this repository are those in capnproto commit
ad4079b (master at the time of writing). Unfortunately, the stable
release is _very_ old, and some schemas out in the wild (notably
sandstorm) have started expecting more recent versions of the base
schemas.

# Directory Structure

The directory structure of this repository is designed such that when
compiling other schema, it should be sufficient to execute:

    capnp compile -I ${path_to_this_repository}/std -ogo ${schama_name}.capnp

And have the `$import` statements in existing schema "just work."

To achieve this, the base schemas themselves are stored as
`/std/capnp/${schema_name}.capnp`. The generated go source files are
stored in a subdirectory, to make them their own package:
`/std/capnp/${mangled_schema_name}/${mangled_schema_name}.capnp.go`.

In addition to the upstream base schemas, we also ship a schema
`/std/go.capnp`, which contains annotations used by `go-capnpc`. Its
usage is described in the top-level README. The generated source is
placed in the root of the repository, making it part of the go package
`capnproto.org/go/capnp/v3`.
