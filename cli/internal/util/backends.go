package util

func IsYarn(backendName string) bool {
	return backendName == "nodejs-yarn" || backendName == "nodejs-berry"
}
