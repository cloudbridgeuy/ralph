use crate::cursor_owner::{write_file, write_file_atomic};
use crate::watches::locked;
use ralph_core::worker::{HeadlessEffect, Marker, WorkerEvent, WorkerMarkers};
use std::fs;
use std::sync::{Arc, Mutex};

pub(crate) type Events = Arc<Mutex<Vec<WorkerEvent>>>;

pub(crate) fn push_event(events: &Events, event: WorkerEvent) {
    locked(events).push(event);
}

pub(crate) fn apply_effect(markers: &WorkerMarkers, effect: &HeadlessEffect) {
    let path = |marker| match marker {
        Marker::Ready => &markers.ready,
        Marker::Reported => &markers.reported,
        Marker::Blocked => &markers.blocked,
        Marker::Waiting => &markers.waiting,
    };
    match effect {
        HeadlessEffect::Set(marker) => {
            write_file(path(*marker), "").ok();
        }
        HeadlessEffect::Clear(marker) => {
            fs::remove_file(path(*marker)).ok();
        }
        HeadlessEffect::Publish(content) => {
            write_file_atomic(&markers.blocked, &content.render()).ok();
        }
        HeadlessEffect::ReplyText(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_and_clear_create_and_remove_the_marker_file() {
        let dir = std::env::temp_dir().join(format!("ralph-worker-events-{}", std::process::id()));
        let markers = WorkerMarkers::new(&dir, "A");
        apply_effect(&markers, &HeadlessEffect::Set(Marker::Ready));
        assert!(markers.ready.exists());
        apply_effect(&markers, &HeadlessEffect::Clear(Marker::Ready));
        assert!(!markers.ready.exists());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pushed_events_are_kept_in_order() {
        let events: Events = Arc::default();
        push_event(&events, WorkerEvent::Reported);
        push_event(&events, WorkerEvent::Reported);
        assert_eq!(locked(&events).len(), 2);
    }
}
