package tree_sitter_quilt_test

import (
	"testing"

	tree_sitter "github.com/tree-sitter/go-tree-sitter"
	tree_sitter_quilt "github.com/tree-sitter/tree-sitter-quilt/bindings/go"
)

func TestCanLoadGrammar(t *testing.T) {
	language := tree_sitter.NewLanguage(tree_sitter_quilt.Language())
	if language == nil {
		t.Errorf("Error loading Quilt grammar")
	}
}
