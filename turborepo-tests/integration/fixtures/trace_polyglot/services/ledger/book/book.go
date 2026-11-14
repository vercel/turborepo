// Package book sums the entries of a ledger.
package book

import "example.com/ledger/money"

// Total returns the sum of the entries.
func Total(entries []int64) int64 {
	var sum int64
	for _, e := range entries {
		sum = money.Add(sum, e)
	}
	return sum
}
