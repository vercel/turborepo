package ui

import (
	"os"

	"github.com/fatih/color"
)

type ColorMode int

const (
	ColorModeUndefined ColorMode = iota + 1
	ColorModeSuppressed
	ColorModeForced
)

func GetColorModeFromEnv() ColorMode {
	// The FORCED_COLOR behavior and accepted values are taken from the supports-color NodeJS Package:
	// The accepted values as documented are "0" to disable, and "1", "2", or "3" to force-enable color
	// at the specified support level (1 = 16 colors, 2 = 256 colors, 3 = 16M colors).
	// We don't currently use the level for anything specific, and just treat things as on and off.
	//
	// Note: while "false" and "true" aren't documented, the library coerces these values to 0 and 1
	// respectively, so that behavior is reproduced here as well.
	// https://www.npmjs.com/package/supports-color

	switch forceColor := os.Getenv("FORCE_COLOR"); {
	case forceColor == "false" || forceColor == "0":
		return ColorModeSuppressed
	case forceColor == "true" || forceColor == "1" || forceColor == "2" || forceColor == "3":
		return ColorModeForced
	default:
		return ColorModeUndefined
	}
}

func applyColorMode(colorMode ColorMode) ColorMode {
	switch colorMode {
	case ColorModeForced:
		color.NoColor = false
	case ColorModeSuppressed:
		color.NoColor = true
	case ColorModeUndefined:
	default:
		// color.NoColor already gets its default value based on
		// isTTY and/or the presence of the NO_COLOR env variable.
	}

	if color.NoColor {
		return ColorModeSuppressed
	} else {
		return ColorModeForced
	}
}
