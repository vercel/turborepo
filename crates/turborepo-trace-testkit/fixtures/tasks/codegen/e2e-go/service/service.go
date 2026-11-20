package service

// Getter is what the service needs.
type Getter interface {
	Get(key string) (string, error)
}

// Greet greets the stored name.
func Greet(g Getter) string {
	n, _ := g.Get("name")
	return "hello " + n
}
