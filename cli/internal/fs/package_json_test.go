package fs

import (
	"testing"

	"gotest.tools/v3/assert"
)

func Test_UnmarshalPackageJSON(t *testing.T) {
	type Case struct {
		name           string
		json           string
		expectedFields *PackageJSON
	}

	testCases := []Case{
		{
			name: "basic types are in raw and processed",
			json: `{"name":"foo","version":"1.2.3"}`,
			expectedFields: &PackageJSON{
				Name:    "foo",
				Version: "1.2.3",
				RawJSON: map[string]interface{}{
					"name":    "foo",
					"version": "1.2.3",
				},
			},
		},
		{
			name: "map types get copied",
			json: `{"dependencies":{"foo":"1.2.3"},"devDependencies":{"bar": "^1.0.0"}}`,
			expectedFields: &PackageJSON{
				Dependencies:    map[string]string{"foo": "1.2.3"},
				DevDependencies: map[string]string{"bar": "^1.0.0"},
				RawJSON: map[string]interface{}{
					"dependencies":    map[string]interface{}{"foo": "1.2.3"},
					"devDependencies": map[string]interface{}{"bar": "^1.0.0"},
				},
			},
		},
		{
			name: "array types get copied",
			json: `{"os":["linux", "windows"]}`,
			expectedFields: &PackageJSON{
				Os: []string{"linux", "windows"},
				RawJSON: map[string]interface{}{
					"os": []interface{}{"linux", "windows"},
				},
			},
		},
	}

	for _, testCase := range testCases {
		actual, err := UnmarshalPackageJSON([]byte(testCase.json))
		assert.NilError(t, err, testCase.name)
		assertPackageJSONEqual(t, actual, testCase.expectedFields)
	}
}

func Test_MarshalPackageJSON(t *testing.T) {
	type TestCase struct {
		name     string
		input    *PackageJSON
		expected *PackageJSON
	}

	testCases := []TestCase{
		{
			name: "roundtrip should have no effect",
			input: &PackageJSON{
				Name:    "foo",
				Version: "1.2.3",
				RawJSON: map[string]interface{}{
					"name":    "foo",
					"version": "1.2.3",
				},
			},
			expected: &PackageJSON{
				Name:    "foo",
				Version: "1.2.3",
				RawJSON: map[string]interface{}{
					"name":    "foo",
					"version": "1.2.3",
				},
			},
		},
		{
			name: "structured fields should take priority over raw values",
			input: &PackageJSON{
				Name:    "foo",
				Version: "2.3.4",
				RawJSON: map[string]interface{}{
					"name":    "foo",
					"version": "1.2.3",
				},
			},
			expected: &PackageJSON{
				Name:    "foo",
				Version: "2.3.4",
				RawJSON: map[string]interface{}{
					"name":    "foo",
					"version": "2.3.4",
				},
			},
		},
		{
			name: "empty structured fields don't get serialized",
			input: &PackageJSON{
				Name:    "foo",
				Version: "",
				RawJSON: map[string]interface{}{
					"name":    "foo",
					"version": "1.2.3",
				},
			},
			expected: &PackageJSON{
				Name:    "foo",
				Version: "",
				RawJSON: map[string]interface{}{
					"name": "foo",
				},
			},
		},
		{
			name: "unstructured fields survive the round trip",
			input: &PackageJSON{
				Name: "foo",
				RawJSON: map[string]interface{}{
					"name":          "foo",
					"special-field": "special-value",
					"special-config": map[string]interface{}{
						"flag":  true,
						"value": "toggled",
					},
				},
			},
			expected: &PackageJSON{
				Name: "foo",
				RawJSON: map[string]interface{}{
					"name":          "foo",
					"special-field": "special-value",
					"special-config": map[string]interface{}{
						"flag":  true,
						"value": "toggled",
					},
				},
			},
		},
	}

	for _, testCase := range testCases {
		serializedInput, err := MarshalPackageJSON(testCase.input)
		assert.NilError(t, err, testCase.name)
		actual, err := UnmarshalPackageJSON(serializedInput)
		assert.NilError(t, err, testCase.name)
		assertPackageJSONEqual(t, actual, testCase.expected)
	}
}

// Asserts that the data section of two PackageJSON structs are equal
func assertPackageJSONEqual(t *testing.T, x *PackageJSON, y *PackageJSON) {
	t.Helper()
	assert.Equal(t, x.Name, y.Name)
	assert.Equal(t, x.Version, y.Version)
	assert.DeepEqual(t, x.Scripts, y.Scripts)
	assert.DeepEqual(t, x.Dependencies, y.Dependencies)
	assert.DeepEqual(t, x.DevDependencies, y.DevDependencies)
	assert.DeepEqual(t, x.OptionalDependencies, y.OptionalDependencies)
	assert.DeepEqual(t, x.PeerDependencies, y.PeerDependencies)
	assert.Equal(t, x.PackageManager, y.PackageManager)
	assert.DeepEqual(t, x.Workspaces, y.Workspaces)
	assert.DeepEqual(t, x.Private, y.Private)
	assert.DeepEqual(t, x.RawJSON, y.RawJSON)
}
