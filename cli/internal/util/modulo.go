package util

// PostitiveMod returns a modulo operator like JavaScripts
func PositiveMod(x, d int) int {
	x = x % d
	if x >= 0 {
		return x
	}
	if d < 0 {
		return x - d
	}
	return x + d
}
