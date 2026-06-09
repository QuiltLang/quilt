#!/usr/bin/env python3
"""Demo of the quilt_python bindings. Run after `bin/build-py`:

    PYTHONPATH=. python3 tests/demo.py
"""

from quilt import tb, leaf, sym

expr = (
    tb("binary_operator")
    .c(leaf("integer", "1"))
    .w(" ")
    .c(sym("+"))
    .w(" ")
    .c(leaf("integer", "2"))
    .b()
)
print(expr.coparse())  # -> 1 + 2
