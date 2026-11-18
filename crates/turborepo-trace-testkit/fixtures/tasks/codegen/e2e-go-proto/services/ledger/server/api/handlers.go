package api

import (
	ledgerv1 "example.test/ledger/server/gen/acme/ledger/v1"
)

// Owner names the account of an entry, a generated message type.
func Owner(e *ledgerv1.Entry) string {
	return e.GetAccount()
}
