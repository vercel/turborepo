package api

import (
	"testing"

	ledgerv1 "example.test/ledger/server/gen/acme/ledger/v1"
)

func TestOwner(t *testing.T) {
	if got := Owner(&ledgerv1.Entry{Account: "acct-1"}); got != "acct-1" {
		t.Errorf("Owner() = %q", got)
	}
}
