package rpc

import "capnproto.org/go/capnp/v3/exc"

// MarshalError fills in the fields of e according to err. Returns a non-nil
// error if marshalling fails.
func (e Exception) MarshalError(err error) error {
	e.SetType(Exception_Type(exc.TypeOf(err)))
	return e.SetReason(err.Error())
}
