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
    _char: $ => /[^\\↖↗↙↘↑↓⟨⟩\n]/,
    _non_escape: $ => /\\[^↖↗↙↘↑↓⟨⟩]/,
    escape: $ => /\\[↖↗↙↘↑↓⟨⟩]/,

    newline: $ => /\n/,
    left_quote: $ => /[a-z]*↖/,
    right_quote: $ => "↗",
    left_unquote: $ => /[a-z]*↙/,
    right_unquote: $ => "↘",
    lift: $ => "↑",
    reduce: $ => "↓",
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
