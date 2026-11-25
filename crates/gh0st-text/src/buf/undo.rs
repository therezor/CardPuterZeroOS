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

/// One undo step: the splices of a typing run or a command.
pub(crate) struct Step {
    /// Names the text this step leaves, so undoing back to a saved state
    /// reads as saved.
    pub id: u64,
    pub splices: Vec<Splice>,
}

#[derive(Default)]
pub(crate) struct Undo {
    done: Vec<Step>,
    undone: Vec<Step>,
    /// The last step takes more edits until sealed.
    open: bool,
    next: u64,
    /// Names the text when no step is left: 0 as opened, else the last
    /// step dropped, so undoing to there still reads as changed.
    floor: u64,
}

impl Undo {
    pub fn record(&mut self, s: Splice) {
        self.undone.clear();
        if self.open {
            if let Some(last) = self.done.last_mut().and_then(|g| g.splices.last_mut()) {
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
                g.splices.push(s);
                return;
            }
        }
        self.next += 1;
        self.done.push(Step {
            id: self.next,
            splices: vec![s],
        });
        self.open = true;
        if self.done.len() > STEPS {
            self.floor = self.done.remove(0).id;
        }
    }

    pub fn seal(&mut self) {
        self.open = false;
    }

    /// The text as it stands: the last step's id, 0 for the text as opened.
    pub fn state(&self) -> u64 {
        self.done.last().map_or(self.floor, |g| g.id)
    }

    /// Forgets every step. The text keeps its name.
    pub fn clear(&mut self) {
        self.floor = self.state();
        self.done.clear();
        self.undone.clear();
        self.open = false;
    }

    /// Marks in `used` each file a step's pieces read.
    pub fn mark_stores(&self, used: &mut [bool]) {
        let pieces = self
            .done
            .iter()
            .chain(&self.undone)
            .flat_map(|g| &g.splices)
            .flat_map(|s| s.old.iter().chain(&s.new));
        for p in pieces {
            if let Src::Orig(g) = p.src {
                if let Some(u) = used.get_mut(g as usize) {
                    *u = true;
                }
            }
        }
    }

    pub fn take_undo(&mut self) -> Option<Step> {
        self.open = false;
        self.done.pop()
    }

    pub fn take_redo(&mut self) -> Option<Step> {
        self.open = false;
        self.undone.pop()
    }

    pub fn push_redo(&mut self, g: Step) {
        self.undone.push(g);
    }

    pub fn push_done(&mut self, g: Step) {
        self.done.push(g);
    }

    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }
}
