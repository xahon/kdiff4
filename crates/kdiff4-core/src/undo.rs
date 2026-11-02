use std::ops::Range;


/// One undo step of D: at byte `start`, `removed` was replaced by `inserted`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub start: usize,
    pub removed: String,
    pub inserted: String,
}


impl Step {
    /// The replacement that redoes the step: the range of `removed`, and the text to put there.
    pub fn forward(&self) -> (Range<usize>, &str) {
        (self.start..self.start + self.removed.len(), &self.inserted)
    }


    /// The replacement that undoes the step: the range of `inserted`, and the text to put back.
    pub fn backward(&self) -> (Range<usize>, &str) {
        (self.start..self.start + self.inserted.len(), &self.removed)
    }


    /// The kind of run a one-character edit can continue; `None` for anything else (pastes, line breaks, selections).
    fn run_kind(&self) -> Option<RunKind> {
        let single = |text: &str| {
            let mut chars = text.chars();
            matches!((chars.next(), chars.next()), (Some(c), None) if c != '\n')
        };
        if self.removed.is_empty() && single(&self.inserted) {
            Some(RunKind::Typing)
        } else if self.inserted.is_empty() && single(&self.removed) {
            Some(RunKind::Deleting)
        } else {
            None
        }
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunKind {
    Typing,
    Deleting,
}


/// The undo history of D. Steps before `position` are done, steps after it can be redone.
#[derive(Debug, Clone, Default)]
pub struct History {
    steps: Vec<Step>,
    position: usize,
    /// The position whose text is on disk; `None` once that state can't be reached by undo or redo.
    saved: Option<usize>,
    /// The run the last step is made of, if the next edit may still join it. Undo, redo and save end the run.
    run: Option<RunKind>,
}


impl History {
    /// A history whose starting text is the saved one.
    pub fn new_saved() -> History {
        History { saved: Some(0), ..History::default() }
    }


    /// Records an edit. Typing on one line is one step, and so is a run of Backspace or of Delete.
    pub fn record(&mut self, step: Step) {
        self.steps.truncate(self.position);
        if self.saved.is_some_and(|saved| saved > self.position) {
            self.saved = None;
        }
        let kind = step.run_kind();
        if kind.is_some() && kind == self.run && let Some(last) = self.steps.last_mut() && join(last, &step) {
            return;
        }
        self.steps.push(step);
        self.position = self.steps.len();
        self.run = kind;
    }


    pub fn undo(&mut self) -> Option<&Step> {
        if self.position == 0 {
            return None;
        }
        self.position -= 1;
        self.run = None;
        self.steps.get(self.position)
    }


    pub fn redo(&mut self) -> Option<&Step> {
        let step = self.steps.get(self.position)?;
        self.position += 1;
        self.run = None;
        Some(step)
    }


    pub fn mark_saved(&mut self) {
        self.saved = Some(self.position);
        self.run = None;
    }


    pub fn is_modified(&self) -> bool {
        self.saved != Some(self.position)
    }
}


/// Grows `last` by `next` if `next` continues it in place: typing right after it, Backspace right before it, or Delete at its start.
fn join(last: &mut Step, next: &Step) -> bool {
    if next.removed.is_empty() && next.start == last.start + last.inserted.len() {
        last.inserted.push_str(&next.inserted);
        return true;
    }
    if next.inserted.is_empty() && next.start + next.removed.len() == last.start {
        last.start = next.start;
        last.removed.insert_str(0, &next.removed);
        return true;
    }
    if next.inserted.is_empty() && next.start == last.start {
        last.removed.push_str(&next.removed);
        return true;
    }
    false
}


#[cfg(test)]
mod tests {
    use super::*;


    fn insert(start: usize, text: &str) -> Step {
        Step { start, removed: String::new(), inserted: text.to_string() }
    }


    fn remove(start: usize, text: &str) -> Step {
        Step { start, removed: text.to_string(), inserted: String::new() }
    }


    #[test]
    fn typing_on_one_line_is_one_step() {
        let mut history = History::new_saved();
        history.record(insert(0, "a"));
        history.record(insert(1, "b"));
        history.record(insert(2, "é"));
        assert_eq!(history.undo(), Some(&insert(0, "abé")));
        assert_eq!(history.undo(), None);
    }


    #[test]
    fn newline_paste_and_jumps_start_new_steps() {
        let mut history = History::new_saved();
        history.record(insert(0, "a"));
        history.record(insert(1, "\n"));
        history.record(insert(2, "b"));
        history.record(insert(3, "pasted"));
        history.record(insert(9, "c"));
        history.record(insert(20, "d"));
        assert_eq!(history.undo(), Some(&insert(20, "d")));
        assert_eq!(history.undo(), Some(&insert(9, "c")));
        assert_eq!(history.undo(), Some(&insert(3, "pasted")));
        assert_eq!(history.undo(), Some(&insert(2, "b")));
        assert_eq!(history.undo(), Some(&insert(1, "\n")));
        assert_eq!(history.undo(), Some(&insert(0, "a")));
    }


    #[test]
    fn backspace_and_delete_runs_are_one_step() {
        let mut history = History::new_saved();
        history.record(remove(4, "d"));
        history.record(remove(3, "c"));
        history.record(remove(2, "b"));
        assert_eq!(history.undo(), Some(&remove(2, "bcd")));
        let mut history = History::new_saved();
        history.record(remove(2, "x"));
        history.record(remove(2, "y"));
        assert_eq!(history.undo(), Some(&remove(2, "xy")));
    }


    #[test]
    fn deleting_a_line_break_or_selection_starts_a_new_step() {
        let mut history = History::new_saved();
        history.record(remove(4, "d"));
        history.record(remove(3, "\n"));
        history.record(remove(0, "abc"));
        assert_eq!(history.undo(), Some(&remove(0, "abc")));
        assert_eq!(history.undo(), Some(&remove(3, "\n")));
    }


    #[test]
    fn typing_after_deleting_is_a_new_step() {
        let mut history = History::new_saved();
        history.record(remove(3, "c"));
        history.record(insert(3, "x"));
        assert_eq!(history.undo(), Some(&insert(3, "x")));
        assert_eq!(history.undo(), Some(&remove(3, "c")));
    }


    #[test]
    fn redo_follows_undo_and_new_edit_drops_redo() {
        let mut history = History::new_saved();
        history.record(insert(0, "a"));
        history.undo();
        assert_eq!(history.redo(), Some(&insert(0, "a")));
        assert_eq!(history.redo(), None);
        history.undo();
        history.record(insert(0, "z"));
        assert_eq!(history.redo(), None);
        assert_eq!(history.undo(), Some(&insert(0, "z")));
    }


    #[test]
    fn undo_ends_the_run() {
        let mut history = History::new_saved();
        history.record(insert(0, "a"));
        history.record(insert(5, "x"));
        history.undo();
        history.record(insert(1, "b"));
        assert_eq!(history.undo(), Some(&insert(1, "b")));
        assert_eq!(history.undo(), Some(&insert(0, "a")));
    }


    #[test]
    fn modified_follows_the_saved_position() {
        let mut history = History::new_saved();
        assert!(!history.is_modified());
        history.record(insert(0, "a"));
        assert!(history.is_modified());
        history.mark_saved();
        assert!(!history.is_modified());
        history.record(insert(1, "b"));
        assert!(history.is_modified());
        history.undo();
        assert!(!history.is_modified());
        history.undo();
        assert!(history.is_modified());
        history.record(insert(0, "z"));
        history.undo();
        assert!(history.is_modified());
    }


    #[test]
    fn typing_after_save_is_a_new_step() {
        let mut history = History::new_saved();
        history.record(insert(0, "a"));
        history.mark_saved();
        history.record(insert(1, "b"));
        assert_eq!(history.undo(), Some(&insert(1, "b")));
        assert!(!history.is_modified());
    }


    #[test]
    fn forward_and_backward_ranges() {
        let step = Step { start: 3, removed: "xy".to_string(), inserted: "abc".to_string() };
        assert_eq!(step.forward(), (3..5, "abc"));
        assert_eq!(step.backward(), (3..6, "xy"));
    }
}
