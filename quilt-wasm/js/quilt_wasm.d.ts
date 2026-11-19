// Types for the quilt-wasm runtime (js/runtime.js). assemble.mjs installs this
// as the package's quilt_wasm.d.ts, adding the web build's init() below it.

/** A single string command: `write(s)`, `NL()`, `push(s)` or `POP()`. */
export class WasmStrCmd {
  private constructor();
}

/** A `StrCmd` or a child placeholder (`HOLE()`), for `quote`/`unquote` cmds. */
export class WasmCmdOrHole {
  private constructor();
}

/** A quilt term: immutable, and owned by the garbage collector. */
export class WasmQTerm {
  private constructor();
  /** Serialize the term back to source code. */
  coparse(): string;
  toString(): string;
  /** A no-op: kept for code written when a term was a wasm handle. */
  free(): void;
  [Symbol.dispose](): void;
}

/** A fluent term builder; every method but `b` returns the same builder. */
export class WasmBuilder {
  private constructor();
  /** Splice a child term. */
  c(child: WasmQTerm): WasmBuilder;
  /** Splice a child term (the same as `c`; what expanded code spells). */
  t(child: WasmQTerm): WasmBuilder;
  /** Emit a child term. */
  e(child: WasmQTerm): WasmBuilder;
  /** Write literal source text. */
  w(s: string): WasmBuilder;
  /** Emit a newline (respecting the current prefix). */
  n(): WasmBuilder;
  /** Push an indentation prefix. */
  p(s: string): WasmBuilder;
  /** Pop an indentation prefix. */
  x(): WasmBuilder;
  /** Build the term. */
  b(): WasmQTerm;
  free(): void;
  [Symbol.dispose](): void;
}

/** Start building a tuple node with the given tag. */
export function tb(tag: string): WasmBuilder;
/** A leaf node: a tag whose only content is `code`. */
export function leaf(tag: string, code: string): WasmQTerm;
/** A symbol: a leaf whose tag and code are the same. */
export function sym(s: string): WasmQTerm;
/** An identifier term (the `⟨N⟩` operator). */
export function name(s: string): WasmQTerm;
/** A quoted fragment. */
export function quote(tag: string, index: number, lang: string, term: WasmQTerm, cmds: WasmCmdOrHole[]): WasmQTerm;
/** An unquoted splice. */
export function unquote(tag: string, index: number, lang: string, term: WasmQTerm, cmds: WasmCmdOrHole[]): WasmQTerm;
/** Wrap a `StrCmd` as a `CmdOrHole`. */
export function cmd(c: WasmStrCmd): WasmCmdOrHole;
/** A `Write` command. */
export function write(s: string): WasmStrCmd;
/** A `Push` command. */
export function push(s: string): WasmStrCmd;
/** The `NewLine` command. */
export function NL(): WasmStrCmd;
/** The `Pop` command. */
export function POP(): WasmStrCmd;
/** A child placeholder. */
export function HOLE(): WasmCmdOrHole;

/**
 * Lift a JS value to a term that reconstructs it (the homogeneous `↑`):
 * a number, string or boolean to its literal, a term to the TypeScript that
 * rebuilds it.
 */
export function qlift(value: number | string | boolean | WasmQTerm): WasmQTerm;
/**
 * Lift a JS value to an HTML term: a string to entity-escaped text, a term
 * unchanged.
 */
export function qlift_html(value: number | string | boolean | WasmQTerm): WasmQTerm;

/** A text dialect: any rule left out is JSON's (see `qlift_text`). */
export interface TextDialect {
  string?(s: string): string;
  in_quotes?(s: string): string;
  boolean?(b: boolean): string;
  float?(f: number): string;
  list_open?: string;
  list_sep?: string;
  list_close?: string;
}
/** A value in a text quote: a scalar, a term, or an array of them. */
export type TextValue = number | string | boolean | WasmQTerm | TextValue[];
/**
 * Lift a JS value into a text quote: JSON's spelling unless `dialect`
 * overrides a rule.
 */
export function qlift_text(value: TextValue, dialect?: TextDialect): WasmQTerm;
/**
 * `qlift_text` for a hole between two `"` the author wrote: a string as its
 * escaped contents, an array as its elements joined by the separator.
 */
export function qlift_text_in_quotes(value: TextValue, dialect?: TextDialect): WasmQTerm;
