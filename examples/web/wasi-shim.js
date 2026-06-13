// A minimal WASI preview1 shim — exactly the imports `quilt-expand-wasm` needs
// (argv, env, the std fds, clock, random, proc_exit). Enough to run the Quilt
// expander as a one-shot command: feed the `.quilt` source on stdin, read the
// expanded source off stdout. Zero dependencies; runs in browsers and Node 20+
// (both provide globalThis.crypto, TextEncoder).
//
// Not a general WASI implementation — no real filesystem, no preopens
// (fd_prestat_get returns BADF), std streams are unseekable (fd_seek → SPIPE).

const SUCCESS = 0;
const BADF = 8;
const SPIPE = 70;

class ExitError extends Error {
  constructor(code) {
    super(`exit ${code}`);
    this.code = code;
  }
}

function concat(chunks) {
  let n = 0;
  for (const c of chunks) n += c.length;
  const out = new Uint8Array(n);
  let p = 0;
  for (const c of chunks) {
    out.set(c, p);
    p += c.length;
  }
  return out;
}

export class WASI {
  constructor({ args = [], env = [], stdin = new Uint8Array() } = {}) {
    this.args = args;
    this.env = env;
    this.stdinBuf = stdin;
    this.stdinPos = 0;
    this.stdoutChunks = [];
    this.stderrChunks = [];
    this.memory = null;
    this.exitCode = null;
    this.enc = new TextEncoder();
  }

  get stdoutBytes() {
    return concat(this.stdoutChunks);
  }
  get stderrBytes() {
    return concat(this.stderrChunks);
  }

  // Instantiate must wire `wasiImport` as the `wasi_snapshot_preview1` import.
  get wasiImport() {
    const w = this;
    const dv = () => new DataView(w.memory.buffer);
    const mem = () => new Uint8Array(w.memory.buffer);
    const writeCStrings = (ptrArr, buf, list) => {
      const d = dv();
      const m = mem();
      let p = buf;
      list.forEach((s, i) => {
        d.setUint32(ptrArr + i * 4, p, true);
        const b = w.enc.encode(s);
        m.set(b, p);
        p += b.length;
        m[p++] = 0;
      });
      return SUCCESS;
    };
    const sizes = (cntPtr, bufSizePtr, list) => {
      const d = dv();
      d.setUint32(cntPtr, list.length, true);
      let size = 0;
      for (const s of list) size += w.enc.encode(s).length + 1;
      d.setUint32(bufSizePtr, size, true);
      return SUCCESS;
    };

    return {
      args_sizes_get: (cnt, bufSize) => sizes(cnt, bufSize, w.args),
      args_get: (ptrArr, buf) => writeCStrings(ptrArr, buf, w.args),
      environ_sizes_get: (cnt, bufSize) => sizes(cnt, bufSize, w.env),
      environ_get: (ptrArr, buf) => writeCStrings(ptrArr, buf, w.env),

      clock_time_get: (_id, _precision, timePtr) => {
        dv().setBigUint64(timePtr, BigInt(Date.now()) * 1_000_000n, true);
        return SUCCESS;
      },
      random_get: (ptr, len) => {
        const buf = mem().subarray(ptr, ptr + len);
        for (let off = 0; off < len; off += 65536) {
          globalThis.crypto.getRandomValues(buf.subarray(off, Math.min(off + 65536, len)));
        }
        return SUCCESS;
      },
      proc_exit: (code) => {
        w.exitCode = code;
        throw new ExitError(code);
      },

      fd_close: () => SUCCESS,
      fd_seek: () => SPIPE, // std streams are not seekable
      fd_prestat_get: () => BADF, // no preopened directories
      fd_prestat_dir_name: () => BADF,
      fd_fdstat_get: (_fd, ptr) => {
        const d = dv();
        d.setUint8(ptr, 2); // fs_filetype = CHARACTER_DEVICE
        d.setUint16(ptr + 2, 0, true); // fs_flags
        d.setBigUint64(ptr + 8, 0xffffffffffffffffn, true); // rights_base
        d.setBigUint64(ptr + 16, 0xffffffffffffffffn, true); // rights_inheriting
        return SUCCESS;
      },

      fd_read: (fd, iovs, iovsLen, nreadPtr) => {
        if (fd !== 0) return BADF;
        const d = dv();
        const m = mem();
        let read = 0;
        for (let i = 0; i < iovsLen; i++) {
          const base = d.getUint32(iovs + i * 8, true);
          const len = d.getUint32(iovs + i * 8 + 4, true);
          const remaining = w.stdinBuf.length - w.stdinPos;
          const n = Math.min(len, remaining);
          if (n > 0) {
            m.set(w.stdinBuf.subarray(w.stdinPos, w.stdinPos + n), base);
            w.stdinPos += n;
            read += n;
          }
          if (n < len) break; // stdin exhausted
        }
        d.setUint32(nreadPtr, read, true);
        return SUCCESS;
      },
      fd_write: (fd, iovs, iovsLen, nwrittenPtr) => {
        if (fd !== 1 && fd !== 2) return BADF;
        const d = dv();
        const m = mem();
        const sink = fd === 2 ? w.stderrChunks : w.stdoutChunks;
        let written = 0;
        for (let i = 0; i < iovsLen; i++) {
          const base = d.getUint32(iovs + i * 8, true);
          const len = d.getUint32(iovs + i * 8 + 4, true);
          sink.push(m.slice(base, base + len));
          written += len;
        }
        d.setUint32(nwrittenPtr, written, true);
        return SUCCESS;
      },
    };
  }

  // Run the command to completion, returning its exit code. `_start` traps via
  // ExitError on proc_exit; a clean return is exit 0.
  start(instance) {
    this.memory = instance.exports.memory;
    try {
      instance.exports._start();
    } catch (e) {
      if (e instanceof ExitError) return e.code;
      throw e;
    }
    return this.exitCode ?? 0;
  }
}
