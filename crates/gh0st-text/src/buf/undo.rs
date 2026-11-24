//! Undo as inverse splices: each step remembers the pieces it took out and
//! put in. Pieces point at bytes that never change (the file as opened and
//! the add buffer), so a step is a few words however much text it moved.

use super::{Piece, Src};

/// Steps kept. Older ones go.
const STEPS: usize = 1000;

pub(crate) struct Splice {
    pub at: u64,
    pub old: Vec<Piece>,
    pub old_len: u64,
    pub old_nl: Option<u64>,
    pub new: Vec<Piece>,
    pub new_len: u64,
    pub new_nl: Option<u64>,
}

#[derive(Default)]
pub(crate) struct Undo {
    done: Vec<Vec<Splice>>,
    undone: Vec<Vec<Splice>>,
    /// The last step takes more edits until sealed.
    open: bool,
}

impl Undo {
    pub fn record(&mut self, s: Splice) {
        self.undone.clear();
        if self.open {
            if let Some(last) = self.done.last_mut().and_then(|g| g.last_mut()) {
                // Typing on from the last insert: one splice, not a hundred.
                if last.old_len == 0 && s.old_len == 0 && s.at == last.at + last.new_len {
                    last.new_len += s.new_len;
                    last.new_nl = last.new_nl.zip(s.new_nl).map(|(a, b)| a + b);
                    for p in s.new {
                        match last.new.last_mut() {
                            Some(q)
                                if q.src == Src::Add
                                    && p.src == Src::Add
                                    && q.off + q.len == p.off =>
                            {
                                q.len += p.len
                            }
                            _ => last.new.push(p),
                        }
                    }
                    return;
                }
            }
            if let Some(g) = self.done.last_mut() {
                g.push(s);
                return;
            }
        }
        self.done.push(vec![s]);
        self.open = true;
        if self.done.len() > STEPS {
            self.done.remove(0);
        }
    }

    pub fn seal(&mut self) {
        self.open = false;
    }

    pub fn take_undo(&mut self) -> Option<Vec<Splice>> {
        self.open = false;
        self.done.pop()
    }

    pub fn take_redo(&mut self) -> Option<Vec<Splice>> {
        self.open = false;
        self.undone.pop()
    }

    pub fn push_redo(&mut self, g: Vec<Splice>) {
        self.undone.push(g);
    }

    pub fn push_done(&mut self, g: Vec<Splice>) {
        self.done.push(g);
    }

    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }
}
