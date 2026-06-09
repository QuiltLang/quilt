//! Bidirectional byte-offset map between a `.quilt` document and a projected
//! virtual document.
//!
//! A projection is a sequence of segments, each either **copied** verbatim from
//! the quilt source (byte offsets shift by a constant within the segment) or
//! **synthetic** text inserted into the virtual document with no quilt
//! counterpart (placeholders, wrapper prologues). Quilt bytes that don't appear
//! in the virtual document at all (brackets, other-language regions) are simply
//! absent — a quilt offset there maps to `None`, which is exactly the signal the
//! router uses to decide a position belongs to a different language.

/// Source of a virtual segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Src {
    /// Bytes copied from the quilt source starting at this offset; length and
    /// content match the virtual segment, so mapping is linear within it.
    Copy { quilt_start: usize },
    /// Synthetic text; maps back to a single anchor quilt offset.
    Synth { anchor: usize },
}

#[derive(Debug, Clone, Copy)]
struct Seg {
    virt_start: usize,
    len: usize,
    src: Src,
}

/// A copied span indexed by its quilt offset, for the quilt→virtual direction.
#[derive(Debug, Clone, Copy)]
struct Copy {
    quilt_start: usize,
    len: usize,
    virt_start: usize,
}

#[derive(Debug, Clone, Default)]
pub struct SourceMap {
    /// Segments in virtual-offset order (contiguous, covering the virtual doc).
    segs: Vec<Seg>,
    /// Copied spans in quilt-offset order.
    copies: Vec<Copy>,
}

impl SourceMap {
    /// Map a virtual byte offset back to a quilt byte offset. Synthetic spans
    /// collapse to their anchor.
    pub fn virtual_to_quilt(&self, voff: usize) -> usize {
        if self.segs.is_empty() {
            return 0;
        }
        let i = match self.segs.binary_search_by(|s| s.virt_start.cmp(&voff)) {
            Ok(i) => i,
            Err(0) => 0,
            Err(i) => i - 1,
        };
        let s = self.segs[i];
        match s.src {
            Src::Copy { quilt_start } => quilt_start + (voff - s.virt_start).min(s.len),
            Src::Synth { anchor } => anchor,
        }
    }

    /// Map a quilt byte offset to a virtual byte offset, or `None` if that quilt
    /// byte isn't present in this projection (a masked / other-language span).
    pub fn quilt_to_virtual(&self, qoff: usize) -> Option<usize> {
        if self.copies.is_empty() {
            return None;
        }
        let i = match self.copies.binary_search_by(|c| c.quilt_start.cmp(&qoff)) {
            Ok(i) => i,
            Err(0) => return None,
            Err(i) => i - 1,
        };
        let c = self.copies[i];
        // Inclusive of the end boundary so end-exclusive ranges map cleanly.
        if qoff <= c.quilt_start + c.len {
            Some(c.virt_start + (qoff - c.quilt_start))
        } else {
            None
        }
    }
}

/// Builds a virtual document and its [`SourceMap`] together.
#[derive(Debug, Default)]
pub struct Builder {
    text: String,
    raw: Vec<(usize, Option<usize>)>, // (len, quilt_start) in virtual order
}

impl Builder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append `s`, copied verbatim from quilt offset `quilt_start`.
    pub fn copy(&mut self, quilt_start: usize, s: &str) {
        if s.is_empty() {
            return;
        }
        self.raw.push((s.len(), Some(quilt_start)));
        self.text.push_str(s);
    }

    /// Append synthetic text with no quilt counterpart.
    pub fn synth(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        self.raw.push((s.len(), None));
        self.text.push_str(s);
    }

    /// Current length of the virtual document being built, in bytes.
    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Finish, computing synthetic anchors and the quilt-order copy index.
    pub fn finish(self) -> (String, SourceMap) {
        let n = self.raw.len();

        // For each synthetic seg, the quilt offset of the next copied seg (or,
        // failing that, the end of the previous copied seg) is its anchor.
        let mut next_copy = vec![None; n];
        let mut acc = None;
        for i in (0..n).rev() {
            if let Some(q) = self.raw[i].1 {
                acc = Some(q);
            }
            next_copy[i] = acc;
        }
        let mut prev_end = vec![None; n];
        let mut accp = None;
        for (slot, &(len, quilt_start)) in prev_end.iter_mut().zip(self.raw.iter()) {
            *slot = accp;
            if let Some(q) = quilt_start {
                accp = Some(q + len);
            }
        }

        let mut segs = Vec::with_capacity(n);
        let mut copies = Vec::new();
        let mut virt = 0usize;
        for (i, &(len, quilt_start)) in self.raw.iter().enumerate() {
            let src = match quilt_start {
                Some(q) => {
                    copies.push(Copy {
                        quilt_start: q,
                        len,
                        virt_start: virt,
                    });
                    Src::Copy { quilt_start: q }
                }
                None => Src::Synth {
                    anchor: next_copy[i].or(prev_end[i]).unwrap_or(0),
                },
            };
            segs.push(Seg {
                virt_start: virt,
                len,
                src,
            });
            virt += len;
        }

        // `copies` is built in virtual order; appended fragments make that
        // differ from quilt order. The quilt→virtual lookup binary-searches by
        // quilt offset, so sort here. Copied quilt ranges are disjoint (masked
        // spans are synthetic, never copied), so the sort is unambiguous.
        copies.sort_by_key(|c| c.quilt_start);

        (self.text, SourceMap { segs, copies })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_roundtrip() {
        let src = "hello world";
        let mut b = Builder::new();
        b.copy(0, src);
        let (text, map) = b.finish();
        assert_eq!(text, src);
        for q in 0..=src.len() {
            assert_eq!(map.quilt_to_virtual(q), Some(q));
            assert_eq!(map.virtual_to_quilt(q), q);
        }
    }

    #[test]
    fn synth_in_the_middle() {
        // quilt:   "ab[XY]cd"  where [XY] (offsets 2..6) is masked, replaced by
        // a 1-char synth "_" in the virtual doc.
        let quilt = "abXYcd";
        let mut b = Builder::new();
        b.copy(0, "ab"); // quilt 0..2 -> virt 0..2
        b.synth("_"); //    masks quilt 2..4 -> virt 2..3
        b.copy(4, "cd"); // quilt 4..6 -> virt 3..5
        let (text, map) = b.finish();
        assert_eq!(text, "ab_cd");

        // Ground bytes map across the synth gap.
        assert_eq!(map.quilt_to_virtual(0), Some(0));
        assert_eq!(map.quilt_to_virtual(2), Some(2)); // boundary into the mask
        assert_eq!(map.quilt_to_virtual(4), Some(3)); // first byte after mask
        assert_eq!(map.quilt_to_virtual(6), Some(5)); // end

        // A byte strictly inside the masked region has no virtual position.
        assert_eq!(map.quilt_to_virtual(3), None);

        // The synthetic byte maps back to the mask's quilt boundary (anchor =
        // next copy's quilt_start == 4).
        assert_eq!(map.virtual_to_quilt(2), 4);

        // Copied virtual bytes map back exactly.
        assert_eq!(map.virtual_to_quilt(0), 0);
        assert_eq!(map.virtual_to_quilt(3), 4);
        assert_eq!(map.virtual_to_quilt(5), 6);
        let _ = quilt;
    }

    #[test]
    fn synth_at_start_anchors_forward() {
        let mut b = Builder::new();
        b.synth("PRE");
        b.copy(0, "x");
        let (text, map) = b.finish();
        assert_eq!(text, "PREx");
        // Synthetic prologue maps back to the start of real content.
        assert_eq!(map.virtual_to_quilt(0), 0);
        assert_eq!(map.virtual_to_quilt(3), 0);
    }
}
