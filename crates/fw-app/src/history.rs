//! Undo/redo of chart edits (snapshots of the notes, beat grid and sections).

use fw_project::{BeatGrid, Note, Project, SongSection};

type Snapshot = (Vec<Note>, BeatGrid, Vec<SongSection>);

fn swap_in(p: &mut Project, (notes, grid, sections): Snapshot) -> Snapshot {
    (std::mem::replace(&mut p.notes, notes), std::mem::replace(&mut p.grid, grid), std::mem::replace(&mut p.sections, sections))
}

#[derive(Default)]
pub struct History {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
}

const LIMIT: usize = 200;

impl History {
    /// Records the state before an edit.
    pub fn push(&mut self, p: &Project) {
        self.undo.push((p.notes.clone(), p.grid.clone(), p.sections.clone()));
        if self.undo.len() > LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn undo(&mut self, p: &mut Project) -> bool {
        let Some(snapshot) = self.undo.pop() else { return false };
        self.redo.push(swap_in(p, snapshot));
        true
    }

    pub fn redo(&mut self, p: &mut Project) -> bool {
        let Some(snapshot) = self.redo.pop() else { return false };
        self.undo.push(swap_in(p, snapshot));
        true
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
    fn undo_and_redo_restore_sections() {
        let mut p = Project::new("/tmp/x", "a.mp3");
        let mut h = History::default();
        h.push(&p);
        p.set_section(1.0, "verse");
        assert!(h.undo(&mut p));
        assert!(p.sections.is_empty());
        assert!(h.redo(&mut p));
        assert_eq!(p.sections.len(), 1);
    }

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
