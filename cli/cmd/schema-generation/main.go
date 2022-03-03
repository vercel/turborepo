package main

import (
	"fmt"
	"strings"

	"github.com/invopop/jsonschema"
	"github.com/vercel/turborepo/cli/internal/fs"
)

func main() {
	r := new(jsonschema.Reflector)

	if err := r.AddGoComments("github.com/vercel/turborepo/cli", "internal/fs"); err != nil {
		panic(err)
	}

	for key, value := range r.CommentMap {
		r.CommentMap[key] = applySoftLineBreaks(value)
	}

	var schema = r.Reflect(&fs.TurboConfigJSON{})
	var schemajson, err = schema.MarshalJSON()
	if err != nil {
		panic(err)
	}
	fmt.Println(string(schemajson))
}

// applySoftLineBreaks is a function that replaces all soft line breaks
// with a space, and hard line breaks with a newline.
func applySoftLineBreaks(comment string) string {
	replaced := strings.ReplaceAll(comment, "\n\n", "[[newline]]")
	replaced = strings.ReplaceAll(replaced, "\n", " ")
	replaced = strings.ReplaceAll(replaced, "[[newline]]", "\n")
	return replaced
}
