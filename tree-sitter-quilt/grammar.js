// Quilt's own block comment, `⟨/*⟩ … ⟨*/⟩`. A function rather than a rule
// because both `_comment` and `_bounded_comment` are `token(choice(…))`, and a
// `token()` may not reference another rule — so the body has to be inlined into
// each of them.
const blockComment = () => seq(
  optional(/\n\s*/),
  '⟨/*⟩',
  repeat(choice(
    /[^⟨]/,       // match exactly 0 chars of "⟨*/⟩"
    /⟨[^*]/,      // match exactly 1 chars of "⟨*/⟩"
    /⟨\*[^/]/,    // match exactly 2 chars of "⟨*/⟩"
    /⟨\*\/[^⟩]/,  // match exactly 3 chars of "⟨*/⟩"
  )),
  '⟨*/⟩',         // match exactly 4 chars of "⟨*/⟩"
);

// This grammar is the *specification* of Quilt's surface syntax, and what
// quilt-lsp's `regions` and the VS Code extension parse with. It is no longer
// what `Node::parse` runs: since issue #254 that is a hand-written scanner in
// quilt/src/node/parse.rs. The two must agree, and
// quilt/tests/parser_differential.rs runs both over a shared corpus to make
// sure they do — so a change here needs the scanner changed with it, and that
// test will say so.
module.exports = grammar({
  name: 'quilt',
  extras: $ => [], // NOTE: don't remove this
  rules: {
    source_file: $ => repeat($._node),

    // Two node sets, differing only in how far a *line* comment may run.
    //
    // At ground level it runs to end of line, closing arrows and all: prose in
    // a comment routinely mentions `↖↗` and `↙…↘` (four files under examples/
    // do), and there is no bracket for it to break out of.
    //
    // Inside `↖…↗` / `↙…↘` there is, and before issue #226 the comment ate it:
    // `rs↖let x = 1; // hi↗;` lexed the `↗` as comment text and the quote was
    // never closed. So the bracketed set uses line-comment tokens that stop at
    // a closing arrow. They are distinct tokens rather than one token with a
    // context flag because that is how tree-sitter's context-aware lexer can
    // tell the two apart: only one of them is valid in any given parse state.
    _node: $ => choice(
      $.content,
      $.escape,
      $.newline,
      $.quote,
      $.unquote,
      $.lift,
      $.reduce,
      $.emit,
      $.type,
      $.name,
      // NOTE: comment is not an "extra" because we don't want it inside (content) nodes
      $._comment,
      $.plain_line_comment,
      $.plain_block_comment,
    ),

    // Same, inside brackets. The line comments are aliased back to the ground
    // spelling so the parse tree — and `Node::from_ts` in quilt/src/node/ts.rs —
    // sees one node kind either way.
    _bracketed_node: $ => choice(
      $.content,
      $.escape,
      $.newline,
      $.quote,
      $.unquote,
      $.lift,
      $.reduce,
      $.emit,
      $.type,
      $.name,
      $._bounded_comment,
      alias($.bounded_plain_line_comment, $.plain_line_comment),
      $.plain_block_comment,
    ),

    content: $ => prec.right(repeat1(choice($._char, $._non_escape))),
    // NOTE: the three classes below must list the same glyphs, and must match
    // GLYPHS in quilt/src/glyphs.rs — they are the set of characters Quilt gives
    // special meaning to, and hence the set `\` can escape. `←` is included
    // because it is the emit glyph *and* Lean's monadic bind (issue #141).
    _char: $ => /[^\\↖↗↙↘↑↓←⟨⟩\n]/,
    _non_escape: $ => /\\[^↖↗↙↘↑↓←⟨⟩]/,
    escape: $ => /\\[↖↗↙↘↑↓←⟨⟩]/,

    newline: $ => /\n/,
    // NOTE: the three annotated openers below must all spell the language name
    // the same way — a lowercase letter followed by letters or digits, or
    // nothing at all for the un-annotated form. Digits are here because `lean4`
    // is a registered alias (see `metas` in quilt/src/langs/omni.rs) that
    // `[a-z]*` could not express, so `lean4↖…↗` was the content `lean4`
    // followed by an *un-annotated* quote and failed far from the cause
    // (issue #222).
    //
    // The leading letter is required, and that is the whole reason this is not
    // simply `[a-z0-9]*`: a number that happens to abut the glyph must stay
    // content. `x = 42↖…↗` is the literal `42` and a bare quote, not a quote of
    // some language "42" — and the corpus pins it.
    left_quote: $ => /([a-z][a-z0-9]*)?↖/,
    right_quote: $ => "↗",
    left_unquote: $ => /([a-z][a-z0-9]*)?↙/,
    right_unquote: $ => "↘",
    lift: $ => "↑",
    reduce: $ => /([a-z][a-z0-9]*)?↓/,
    emit: $ => "←",
    type: $ => "⟨T⟩",
    name: $ => "⟨N⟩",
    quote: $ => seq($.left_quote, repeat($._bracketed_node), $.right_quote),
    unquote: $ => seq($.left_unquote, repeat($._bracketed_node), $.right_unquote),

    // Plain C-style line comment: passes through to output; Quilt special chars inside are raw text.
    // prec(1) ensures this wins over content when '//' appears at a token boundary.
    plain_line_comment: $ => token(prec(1, seq('//', /.*/))),
    // The bracketed spelling: stops at a closing arrow, which is the one thing
    // it may not swallow (#226). A comment that really needs a `↗` in it can be
    // a block comment, whose explicit terminator runs no such risk.
    bounded_plain_line_comment: $ => token(prec(1, seq('//', /[^↗↘\n]*/))),

    // Plain C-style block comment: passes through to output; Quilt special chars inside are raw text.
    // prec(1) ensures this wins over content when '/*' appears at a token boundary.
    plain_block_comment: $ => token(prec(1, seq('/*', /[^*]*\*+([^/*][^*]*\*+)*/, '/'))),

    _comment: $ => token(choice(
      // Line comments
      seq(optional(/\n\s*/), '⟨//⟩', /.*/),
      blockComment(),
    )),

    // Quilt's own comment, inside brackets: the line form is bounded for the
    // same reason as `bounded_plain_line_comment`. Both are hidden rules, so
    // neither reaches the parse tree and no alias is needed.
    _bounded_comment: $ => token(choice(
      seq(optional(/\n\s*/), '⟨//⟩', /[^↗↘\n]*/),
      blockComment(),
    )),
  }
});
