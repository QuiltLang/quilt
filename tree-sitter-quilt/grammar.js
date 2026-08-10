module.exports = grammar({
  name: 'quilt',
  extras: $ => [], // NOTE: don't remove this
  rules: {
    source_file: $ => repeat($._node),
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

    content: $ => prec.right(repeat1(choice($._char, $._non_escape))),
    // NOTE: the three classes below must list the same glyphs, and must match
    // GLYPHS in quilt/src/node.rs — they are the set of characters Quilt gives
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
    quote: $ => seq($.left_quote, repeat($._node), $.right_quote),
    unquote: $ => seq($.left_unquote, repeat($._node), $.right_unquote),

    // Plain C-style line comment: passes through to output; Quilt special chars inside are raw text.
    // prec(1) ensures this wins over content when '//' appears at a token boundary.
    plain_line_comment: $ => token(prec(1, seq('//', /.*/))),

    // Plain C-style block comment: passes through to output; Quilt special chars inside are raw text.
    // prec(1) ensures this wins over content when '/*' appears at a token boundary.
    plain_block_comment: $ => token(prec(1, seq('/*', /[^*]*\*+([^/*][^*]*\*+)*/, '/'))),

    _comment: $ => token(choice(
      // Line comments
      seq(optional(/\n\s*/), '⟨//⟩', /.*/),
      // Block comments
      seq(
        optional(/\n\s*/),
        '⟨/*⟩',
        repeat(choice(
          /[^⟨]/,       // match exactly 0 chars of "⟨*/⟩"
          /⟨[^*]/,      // match exactly 1 chars of "⟨*/⟩"
          /⟨\*[^/]/,    // match exactly 2 chars of "⟨*/⟩"
          /⟨\*\/[^⟩]/,  // match exactly 3 chars of "⟨*/⟩"
        )),
        '⟨*/⟩',         // match exactly 4 chars of "⟨*/⟩"
      ),
    )),
  }
});
