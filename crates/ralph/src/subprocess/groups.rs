use std::collections::BTreeSet;
use std::sync::{Mutex, PoisonError};

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct GroupSet(BTreeSet<u32>);

impl GroupSet {
    pub(crate) const fn new() -> Self {
        Self(BTreeSet::new())
    }

    pub(crate) fn register(&mut self, group: u32) {
        self.0.insert(group);
    }

    pub(crate) fn unregister(&mut self, group: u32) {
        self.0.remove(&group);
    }

    pub(crate) fn snapshot(&self) -> Vec<u32> {
        self.0.iter().copied().collect()
    }
}

static GROUPS: Mutex<GroupSet> = Mutex::new(GroupSet::new());

fn with_groups<T>(apply: impl FnOnce(&mut GroupSet) -> T) -> T {
    apply(&mut GROUPS.lock().unwrap_or_else(PoisonError::into_inner))
}

pub(crate) fn register_group(group: u32) {
    with_groups(|groups| groups.register(group));
}

pub(crate) fn unregister_group(group: u32) {
    with_groups(|groups| groups.unregister(group));
}

pub(crate) fn kill_registered_groups() {
    for group in with_groups(|groups| groups.snapshot()) {
        unsafe { libc::killpg(group as libc::pid_t, libc::SIGKILL) };
    }
}

pub(crate) struct Registered(u32);

impl Registered {
    pub(crate) fn new(group: u32) -> Self {
        register_group(group);
        Self(group)
    }
}

impl Drop for Registered {
    fn drop(&mut self) {
        unregister_group(self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_set_keeps_each_registered_group_once_until_it_is_unregistered() {
        let mut groups = GroupSet::new();
        assert_eq!(groups.snapshot(), Vec::<u32>::new());
        groups.register(30);
        groups.register(10);
        groups.register(30);
        assert_eq!(groups.snapshot(), vec![10, 30]);
        groups.unregister(10);
        groups.unregister(99);
        assert_eq!(groups.snapshot(), vec![30]);
    }
}
