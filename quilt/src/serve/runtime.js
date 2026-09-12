// The quilt runtime, installed into the page's own realm — the machine a
// TypeScript cell runs on in a live notebook.
//
// An expanded TypeScript cell is builder calls (`tb("element").w("<b>")…`)
// and, where it stages, a `.reduce()`. Those have to be names the page
// knows, which is what `install` does. The server prepends `RUNTIME_BUILT`
// to this module, so a page can say honestly whether the wasm half is there.
//
// `↓` (reduce) is the interesting one. Reduce must *expand* a generated
// stage before running it, and the expander is not in the page — in the
// playground it is a WASI module, in the Python runtime it is the `quilt`
// binary. Here it is the notebook server, one hop away over `/expand`. The
// call is synchronous because `term.reduce()` is: a deprecated sync
// XMLHttpRequest is the honest cost of a synchronous operator whose
// implementation lives outside the page, and it is a localhost round trip.

const GLYPHS = "↖↗↙↘↑↓←⟨⟩";
const hasGlyph = (s) => [...GLYPHS].some((g) => s.includes(g));

// The names an expanded stage calls. Kept in step with the wasm runtime's
// exports (the same list `examples/web/quilt-rt.js` passes to its sandbox).
const RT_NAMES = [
  "tb", "leaf", "sym", "quote", "unquote", "cmd", "write", "push",
  "name", "qlift", "qlift_html", "NL", "POP", "HOLE",
];

// Expand Quilt source with the server that is serving this page.
export function expand(lang, src) {
  const request = new XMLHttpRequest();
  request.open("POST", "/expand", false); // sync: `reduce()` has no `await`
  request.setRequestHeader("Content-Type", "application/json");
  request.send(JSON.stringify({ lang, src }));
  if (request.status !== 200) {
    throw new Error(`expand failed: ${request.status} ${request.responseText}`);
  }
  return JSON.parse(request.responseText).program;
}

// Install the runtime as globals of `win`. Answers whether the wasm half
// loaded: without it the page still runs plain TypeScript, and a cell that
// quotes fails with a message saying what to build.
export async function install(win) {
  win.quiltExpand = (src, lang = "ts") => expand(lang, src);
  if (!RUNTIME_BUILT) {
    for (const name of RT_NAMES) {
      if (name in win) continue;
      win[name] = () => {
        throw new Error(
          `this cell quotes, so it needs the browser runtime: \`wasm-pack build quilt-wasm ` +
            `--target web --out-dir pkg-web\`, then reload`,
        );
      };
    }
    return false;
  }
  const RT = await import("/quilt-wasm/quilt_wasm.js");
  await RT.default();
  for (const name of RT_NAMES) {
    if (RT[name] !== undefined) win[name] = RT[name];
  }
  // `term.↓` expands (the TypeScript meta) to `term.reduce()`; the wasm
  // runtime has no reduce, because reduce needs the expander.
  if (RT.WasmQTerm && !RT.WasmQTerm.prototype.reduce) {
    RT.WasmQTerm.prototype.reduce = function reduce() {
      let code = this.coparse();
      if (hasGlyph(code)) code = expand("ts", code);
      code = code
        .split("\n")
        .filter((line) => !line.startsWith("//!"))
        .join("\n");
      const fn = new Function(...RT_NAMES, `"use strict";\nreturn (\n${code}\n);`);
      return fn(...RT_NAMES.map((n) => RT[n]));
    };
  }
  win.quilt = RT;
  return true;
}
