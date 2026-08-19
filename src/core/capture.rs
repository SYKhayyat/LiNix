//! A capture with a ceiling.
//!
//! **Every accumulating path in this tree has a bound except the one that kept the bytes.**
//! `core::download` refuses a body over 2 GiB before it fills the disk, and the terminal mirror
//! caps what it prints — while `RawExecutor::wait_watched`'s `pump` read a child's stdout and
//! stderr into a `Vec` in 8 KiB chunks with no cap of any kind, holding a manager invocation's
//! complete output in memory until the command exited. A `nix` build, a `cargo install`
//! compiling, a progress bar redrawn with carriage returns for an hour: each grows that without
//! limit, and a concurrent wave holds one per running manager.
//!
//! Its own module rather than another hundred lines of `executor.rs`, which is the file
//! `a_module_is_a_subject_not_a_pile` was about — "what a captured stream keeps" is a subject,
//! and it is not the same subject as "how a child is owned and stopped".

/// The most of one child's output Shall keeps in memory, per stream.
///
/// 8 MiB: far past any real diagnostic and far short of a problem. What is kept is the head and
/// the tail, because those are the two parts an error message wants — the invocation that
/// started it and the failure that ended it — and the middle of a three-hour build log is
/// neither.
pub(crate) const MAX_CAPTURED_BYTES: usize = 8 * 1024 * 1024;

/// A capture that keeps the head and the tail of a stream and drops the middle.
///
/// See [`MAX_CAPTURED_BYTES`]. The truncation is announced in the bytes
/// themselves, because what this becomes is an error message a person reads, and a log
/// with a silent hole in it is worse than one that says where the hole is.
pub(crate) struct Capped {
    head: Vec<u8>,
    tail: std::collections::VecDeque<u8>,
    dropped: usize,
}

impl Capped {
    /// Half the budget to each end.
    const HALF: usize = MAX_CAPTURED_BYTES / 2;

    pub(crate) fn new() -> Self {
        Self {
            head: Vec::new(),
            tail: std::collections::VecDeque::new(),
            dropped: 0,
        }
    }

    pub(crate) fn push(&mut self, bytes: &[u8]) {
        let mut bytes = bytes;
        if self.head.len() < Self::HALF {
            let room = Self::HALF - self.head.len();
            let take = room.min(bytes.len());
            self.head.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
        }
        for &b in bytes {
            self.tail.push_back(b);
            if self.tail.len() > Self::HALF {
                self.tail.pop_front();
                self.dropped += 1;
            }
        }
    }

    pub(crate) fn finish(self) -> Vec<u8> {
        if self.dropped == 0 {
            let mut out = self.head;
            out.extend(self.tail);
            return out;
        }
        let mut out = self.head;
        out.extend_from_slice(
            format!(
                "\n… [shall: {} bytes of output omitted; the head and tail are kept] …\n",
                self.dropped
            )
            .as_bytes(),
        );
        out.extend(self.tail);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A child's output is kept head-and-tail, never without bound.**
    ///
    /// B7: `pump` grew a `Vec` for the whole of a child's output until the command exited — a
    /// `nix` build, a `cargo install` compiling, a progress bar redrawn with carriage returns
    /// for an hour — and a concurrent wave holds one per running manager. Every other
    /// accumulating path in this tree has a ceiling; `core::download` refuses a body over 2 GiB
    /// before it fills the disk. This one kept the bytes.
    ///
    /// What is kept is the two parts an error message wants: the invocation that started it and
    /// the failure that ended it. The middle of a three-hour build log is neither, and the
    /// omission says so in the bytes, because a log with a silent hole in it is worse than one
    /// that names the hole.
    #[test]
    fn a_capture_past_the_ceiling_keeps_the_head_and_the_tail() {
        let mut c = Capped::new();
        let head = "START-OF-OUTPUT";
        c.push(head.as_bytes());
        // Well past the ceiling, in chunks, the way `pump` feeds it.
        let filler = vec![b'x'; 64 * 1024];
        for _ in 0..(MAX_CAPTURED_BYTES / filler.len() + 4) {
            c.push(&filler);
        }
        let tail = "END-OF-OUTPUT";
        c.push(tail.as_bytes());

        let out = String::from_utf8_lossy(&c.finish()).to_string();
        assert!(out.starts_with(head), "the head was dropped");
        assert!(
            out.ends_with(tail),
            "the tail was dropped, which is the half an error needs"
        );
        assert!(
            out.contains("bytes of output omitted"),
            "the truncation is silent; a log with an unmarked hole reads as a complete one"
        );
        assert!(
            out.len() <= MAX_CAPTURED_BYTES + 256,
            "the capture is {} bytes against a ceiling of {}",
            out.len(),
            MAX_CAPTURED_BYTES
        );
    }

    /// Below the ceiling nothing changes at all — no marker, no reordering, byte for byte what
    /// the child wrote. A ceiling that alters ordinary output would break every error message
    /// in the program to fix a case almost nobody hits.
    #[test]
    fn a_capture_under_the_ceiling_is_byte_for_byte_what_was_written() {
        let mut c = Capped::new();
        for chunk in ["error: ", "could not resolve ", "ripgrep\n"] {
            c.push(chunk.as_bytes());
        }
        assert_eq!(
            String::from_utf8_lossy(&c.finish()),
            "error: could not resolve ripgrep\n"
        );
    }
}
