from unittest import TestCase

import tree_sitter
import tree_sitter_quilt


class TestLanguage(TestCase):
    def test_can_load_grammar(self):
        try:
            tree_sitter.Language(tree_sitter_quilt.language())
        except Exception:
            self.fail("Error loading Quilt grammar")
