from ._quilt import *
import os
import subprocess
import tempfile

def reduce(term):
    return eval(term.coparse())

def reduce_rs(term):
    """Evaluate a QTerm by running it as Rust code via rust-script (the `rs↓` operator)."""
    import importlib.resources as _ir
    quilt_dir = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

    input_code = term.coparse()
    out_fd, out_path = tempfile.mkstemp()
    os.close(out_fd)
    try:
        script = f"""
//! ```cargo
//! [dependencies]
//! quilt = {{ path = "{quilt_dir}", default-features = false, features = ["rust"] }}
//! postcard = {{ version = "1.1", features = ["alloc"] }}
//! ```
#[allow(unused_imports)]
use quilt::prelude::*;
use std::io::Write;
fn main() -> Result<()> {{
    let output: Arc<QTerm> = {input_code};
    let data = postcard::to_allocvec(&output).unwrap();
    let mut file = std::fs::File::create("{out_path}").unwrap();
    file.write_all(&data).unwrap();
    Ok(())
}}
"""
        with tempfile.NamedTemporaryFile(suffix=".rs", delete=False) as sf:
            sf.write(script.encode())
            script_path = sf.name
        try:
            result = subprocess.run(["rust-script", script_path], check=True)
        finally:
            os.unlink(script_path)
        with open(out_path, "rb") as f:
            data = f.read()
        return from_postcard_bytes(data)
    finally:
        try:
            os.unlink(out_path)
        except OSError:
            pass
