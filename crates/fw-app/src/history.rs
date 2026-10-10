//! Undo/redo of chart edits (notes and beat grid snapshots).

use fw_project::{BeatGrid, Note, Project};

type Snapshot = (Vec<Note>, BeatGrid);

#[derive(Default)]
pub struct History {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
}

const LIMIT: usize = 200;

impl History {
    /// Records the state before an edit.
    pub fn push(&mut self, p: &Project) {
        self.undo.push((p.notes.clone(), p.grid.clone()));
        if self.undo.len() > LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn undo(&mut self, p: &mut Project) -> bool {
        match self.undo.pop() {
            Some((notes, grid)) => {
                self.redo.push((std::mem::replace(&mut p.notes, notes), std::mem::replace(&mut p.grid, grid)));
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self, p: &mut Project) -> bool {
        match self.redo.pop() {
            Some((notes, grid)) => {
                self.undo.push((std::mem::replace(&mut p.notes, notes), std::mem::replace(&mut p.grid, grid)));
                true
            }
            None => false,
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_and_redo_restore_notes() {
        let mut p = Project::new("/tmp/x", "a.mp3");
        let mut h = History::default();
        h.push(&p);
        p.notes.push(Note::new(1.0, 2.0, 40));
        assert!(h.undo(&mut p));
        assert!(p.notes.is_empty());
        assert!(h.redo(&mut p));
        assert_eq!(p.notes.len(), 1);
        assert!(!h.redo(&mut p));
    }
}
