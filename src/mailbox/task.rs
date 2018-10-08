use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::ops::Deref;
use std::sync::Arc;

use super::Mailbox;

#[derive(Clone, Debug)]
pub(super) struct ArcCmp<T>(Arc<T>);

impl<T> ArcCmp<T> {
    pub fn new(inner: Arc<T>) -> Self {
        ArcCmp(inner)
    }
    pub fn into_inner(self) -> Arc<T> {
        self.0
    }
}

impl<T> From<Arc<T>> for ArcCmp<T> {
    fn from(ptr: Arc<T>) -> Self {
        ArcCmp(ptr)
    }
}

impl<T> From<T> for ArcCmp<T> {
    fn from(val: T) -> Self {
        ArcCmp::from(Arc::from(val))
    }
}

impl<T> Deref for ArcCmp<T> {
    type Target = Arc<T>;
    fn deref(&self) -> &Arc<T> {
        &self.0
    }
}

impl<T> PartialEq for ArcCmp<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl<T> Eq for ArcCmp<T> { }

impl<T> PartialOrd for ArcCmp<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for ArcCmp<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        // TODO: Is there a nicer way to compare two Arcs?
        let me = self as &T as *const _ as usize;
        let other = other as &T as *const _ as usize;
        me.cmp(&other)
    }
}

// Note: The order of tasks is significant, as it specifies priority
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum Task {
    Rescan(ArcCmp<Mailbox>),
}

impl Task {
    pub fn rescan(mbox: Arc<Mailbox>) -> Self {
        Task::Rescan(ArcCmp::from(mbox))
    }
    fn perform(self) {
        unimplemented!();
    }
}

// We use BTreeSet, not BinaryHeap even though the BinaryHeap is more natural for priority queues.
// We want to have deduplication and we get it for free here.
#[derive(Debug)]
crate struct Queue(BTreeSet<Task>);

impl Queue {
    pub(super) fn new() -> Self {
        Queue(BTreeSet::new())
    }
    pub(super) fn push(&mut self, task: Task) {
        // We don't care if it was already in there. It'll merge duplicates.
        self.0.insert(task);
    }

    fn pop(&mut self) -> Option<Task> {
        if let Some(task) = self.0.iter().next().cloned() {
            self.0.remove(&task);
            Some(task)
        } else {
            None
        }
    }

    /// One turn of the queue.
    ///
    /// Returns true if there was a task (and it was performed) and false if it was empty.
    pub(super) fn turn(&mut self) -> bool {
        if let Some(task) = self.pop() {
            task.perform();
            true
        } else {
            false
        }
    }
}
