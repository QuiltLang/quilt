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
    reduce: $ => /[a-z]*↓/,
    emit: $ => "←",
    type: $ => "⟨T⟩",
    name: $ => "⟨N⟩",
    quote: $ => seq($.left_quote, repeat($._node), $.right_quote),
    unquote: $ => seq($.left_unquote, repeat($._node), $.right_unquote),

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
