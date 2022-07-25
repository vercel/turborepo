package env

import (
	"testing"

	"gotest.tools/v3/assert"
)

func TestGetHashableEnvPairs_SingleVar(t *testing.T) {
	t.Setenv("lowercase", "stillcool")
	t.Setenv("MY_TEST_VAR", "cool")
	t.Setenv("12345", "numbers")
	t.Setenv("one_MORE", "great")
	println("cool")
	result := GetConfigEnvPairs([]string{"MY_TEST_VAR"})
	assert.DeepEqual(t, result, []string([]string{"MY_TEST_VAR=cool"}))
}

func TestGetHashableEnvPairs_MultiVar(t *testing.T) {
	t.Setenv("lowercase", "stillcool")
	t.Setenv("MY_TEST_VAR", "cool")
	t.Setenv("12345", "numbers")
	t.Setenv("one_MORE", "great")
	println("cool")
	result := GetConfigEnvPairs([]string{"MY_TEST_VAR", "12345", "lowercase"})
	assert.DeepEqual(t, result, []string([]string{"MY_TEST_VAR=cool", "12345=numbers", "lowercase=stillcool"}))
}

func TestGetHashableEnvPairs_NoVar(t *testing.T) {
	t.Setenv("lowercase", "stillcool")
	t.Setenv("MY_TEST_VAR", "cool")
	t.Setenv("12345", "numbers")
	t.Setenv("one_MORE", "great")
	println("cool")
	result := GetConfigEnvPairs([]string{})
	assert.DeepEqual(t, result, []string([]string{}))
}

func TestGetAllFrameworkEnvPairs_Next(t *testing.T) {
	t.Setenv("NEXT_PUBLIC_MY_COOL_VAR", "cool")
	t.Setenv("MY_TEST_VAR", "cool")
	t.Setenv("12345", "numbers")
	t.Setenv("one_MORE", "great")
	println("cool")
	result := GetAllFrameworkEnvPairs()
	assert.DeepEqual(t, result, []string([]string{"NEXT_PUBLIC_MY_COOL_VAR=cool"}))
}

func TestGetAllFrameworkEnvPairs_Nuxt(t *testing.T) {
	t.Setenv("NUXT_ENV_MY_COOL_VAR", "cool")
	t.Setenv("MY_TEST_VAR", "cool")
	t.Setenv("12345", "numbers")
	t.Setenv("one_MORE", "great")
	println("cool")
	result := GetAllFrameworkEnvPairs()
	assert.DeepEqual(t, result, []string([]string{"NUXT_ENV_MY_COOL_VAR=cool"}))
}

func TestGetAllFrameworkEnvPairs_React(t *testing.T) {
	t.Setenv("REACT_APP_MY_COOL_VAR", "cool")
	t.Setenv("MY_TEST_VAR", "cool")
	t.Setenv("12345", "numbers")
	t.Setenv("one_MORE", "great")
	println("cool")
	result := GetAllFrameworkEnvPairs()
	assert.DeepEqual(t, result, []string([]string{"REACT_APP_MY_COOL_VAR=cool"}))
}

func TestGetAllFrameworkEnvPairs_Gatsby(t *testing.T) {
	t.Setenv("GATSBY_MY_COOL_VAR", "cool")
	t.Setenv("MY_TEST_VAR", "cool")
	t.Setenv("12345", "numbers")
	t.Setenv("one_MORE", "great")
	println("cool")
	result := GetAllFrameworkEnvPairs()
	assert.DeepEqual(t, result, []string([]string{"GATSBY_MY_COOL_VAR=cool"}))
}

func TestGetAllFrameworkEnvPairs_Public(t *testing.T) {
	t.Setenv("PUBLIC_MY_COOL_VAR", "cool")
	t.Setenv("MY_TEST_VAR", "cool")
	t.Setenv("12345", "numbers")
	t.Setenv("one_MORE", "great")
	println("cool")
	result := GetAllFrameworkEnvPairs()
	assert.DeepEqual(t, result, []string([]string{"PUBLIC_MY_COOL_VAR=cool"}))
}

func TestGetAllFrameworkEnvPairs_Vue(t *testing.T) {
	t.Setenv("VUE_APP_MY_COOL_VAR", "cool")
	t.Setenv("MY_TEST_VAR", "cool")
	t.Setenv("12345", "numbers")
	t.Setenv("one_MORE", "great")
	println("cool")
	result := GetAllFrameworkEnvPairs()
	assert.DeepEqual(t, result, []string([]string{"VUE_APP_MY_COOL_VAR=cool"}))
}

func TestGetAllFrameworkEnvPairs_Vite(t *testing.T) {
	t.Setenv("VITE_MY_COOL_VAR", "cool")
	t.Setenv("MY_TEST_VAR", "cool")
	t.Setenv("12345", "numbers")
	t.Setenv("one_MORE", "great")
	println("cool")
	result := GetAllFrameworkEnvPairs()
	assert.DeepEqual(t, result, []string([]string{"VITE_MY_COOL_VAR=cool"}))
}

func TestGetAllFrameworkEnvPairs_Redwood(t *testing.T) {
	t.Setenv("REDWOOD_ENV_MY_COOL_VAR", "cool")
	t.Setenv("MY_TEST_VAR", "cool")
	t.Setenv("12345", "numbers")
	t.Setenv("one_MORE", "great")
	println("cool")
	result := GetAllFrameworkEnvPairs()
	assert.DeepEqual(t, result, []string([]string{"REDWOOD_ENV_MY_COOL_VAR=cool"}))
}
func TestGetAllFrameworkEnvPairs_Sanity(t *testing.T) {
	t.Setenv("SANITY_STUDIO_MY_COOL_VAR", "cool")
	t.Setenv("MY_TEST_VAR", "cool")
	t.Setenv("12345", "numbers")
	t.Setenv("one_MORE", "great")
	println("cool")
	result := GetAllFrameworkEnvPairs()
	assert.DeepEqual(t, result, []string([]string{"SANITY_STUDIO_MY_COOL_VAR=cool"}))
}

func TestGetHashableEnvPairs(t *testing.T) {
	t.Setenv("NEXT_PUBLIC_MY_COOL_VAR", "cool")
	t.Setenv("NEXT_PUBLIC_MY_COOL_VAR2", "cool1")
	t.Setenv("NUXT_ENV_MY_COOL_VAR", "cool")
	t.Setenv("REACT_APP_MY_COOL_VAR", "cool")
	t.Setenv("GATSBY_MY_COOL_VAR", "cool")
	t.Setenv("PUBLIC_MY_COOL_VAR", "cool")
	t.Setenv("PUBLIC_MY_COOL_VAR2", "cool1")
	t.Setenv("VUE_APP_MY_COOL_VAR", "cool")
	t.Setenv("VITE_MY_COOL_VAR", "cool")
	t.Setenv("REDWOOD_ENV_MY_COOL_VAR", "cool")
	t.Setenv("SANITY_STUDIO_MY_COOL_VAR", "cool")
	t.Setenv("lowercase", "stillcool")
	t.Setenv("MY_TEST_VAR", "cool")
	t.Setenv("12345", "numbers")
	t.Setenv("one_MORE", "great")

	result := GetHashableEnvPairs([]string{"MY_TEST_VAR", "lowercase", "one_MORE"})
	assert.DeepEqual(t, result, []string([]string{"GATSBY_MY_COOL_VAR=cool", "MY_TEST_VAR=cool", "NEXT_PUBLIC_MY_COOL_VAR2=cool1", "NEXT_PUBLIC_MY_COOL_VAR=cool", "NUXT_ENV_MY_COOL_VAR=cool", "PUBLIC_MY_COOL_VAR2=cool1", "PUBLIC_MY_COOL_VAR=cool", "REACT_APP_MY_COOL_VAR=cool", "REDWOOD_ENV_MY_COOL_VAR=cool", "SANITY_STUDIO_MY_COOL_VAR=cool", "VITE_MY_COOL_VAR=cool", "VUE_APP_MY_COOL_VAR=cool", "lowercase=stillcool", "one_MORE=great"}))
}
