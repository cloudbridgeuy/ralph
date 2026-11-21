use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

/// Manages a collection of threads.
///
/// A new thread is created every time all the existing threads are full.
/// Any idle thread will automatically die after a few seconds.
pub struct TaskPool {
    sharing: Arc<Sharing>,
}

struct Sharing {
    // list of the tasks to be done by worker threads
    todo: Mutex<VecDeque<Box<dyn FnMut() + Send>>>,

    // condvar that will be notified whenever a task is added to `todo`
    condvar: Condvar,

    // number of total worker threads running
    active_tasks: AtomicUsize,

    // number of idle worker threads
    waiting_tasks: AtomicUsize,
}

/// Minimum number of active threads.
static MIN_THREADS: usize = 4;

struct Registration<'a> {
    nb: &'a AtomicUsize,
}

impl<'a> Registration<'a> {
    fn new(nb: &'a AtomicUsize) -> Registration<'a> {
        nb.fetch_add(1, Ordering::Release);
        Registration { nb }
    }
}

impl<'a> Drop for Registration<'a> {
    fn drop(&mut self) {
        self.nb.fetch_sub(1, Ordering::Release);
    }
}

impl TaskPool {
    pub fn new() -> TaskPool {
        let pool = TaskPool {
            sharing: Arc::new(Sharing {
                todo: Mutex::new(VecDeque::new()),
                condvar: Condvar::new(),
                active_tasks: AtomicUsize::new(0),
                waiting_tasks: AtomicUsize::new(0),
            }),
        };

        for _ in 0..MIN_THREADS {
            pool.add_thread(None)
        }

        pool
    }

    /// Executes a function in a thread.
    /// If no thread is available, spawns a new one.
    pub fn spawn(&self, code: Box<dyn FnMut() + Send>) {
        let mut queue = self.sharing.todo.lock().unwrap();
        self.dispatch(&mut queue, code);
    }

    fn dispatch(
        &self,
        queue: &mut VecDeque<Box<dyn FnMut() + Send>>,
        code: Box<dyn FnMut() + Send>,
    ) {
        let waiting = self.sharing.waiting_tasks.load(Ordering::Acquire);
        if needs_thread(waiting, queue.len()) {
            self.add_thread(Some(code));
        } else {
            queue.push_back(code);
            self.sharing.condvar.notify_one();
        }
    }

    fn add_thread(&self, initial_fn: Option<Box<dyn FnMut() + Send>>) {
        let sharing = self.sharing.clone();

        thread::spawn(move || {
            let sharing = sharing;
            let _active_guard = Registration::new(&sharing.active_tasks);

            if let Some(mut f) = initial_fn {
                f();
            }

            loop {
                let mut task: Box<dyn FnMut() + Send> = {
                    let mut todo = sharing.todo.lock().unwrap();

                    let task;
                    loop {
                        if let Some(poped_task) = todo.pop_front() {
                            task = poped_task;
                            break;
                        }
                        let _waiting_guard = Registration::new(&sharing.waiting_tasks);

                        let received =
                            if sharing.active_tasks.load(Ordering::Acquire) <= MIN_THREADS {
                                todo = sharing.condvar.wait(todo).unwrap();
                                true
                            } else {
                                let (new_lock, waitres) = sharing
                                    .condvar
                                    .wait_timeout(todo, Duration::from_millis(5000))
                                    .unwrap();
                                todo = new_lock;
                                !waitres.timed_out()
                            };

                        if !received && todo.is_empty() {
                            return;
                        }
                    }

                    task
                };

                task();
            }
        });
    }
}

impl Drop for TaskPool {
    fn drop(&mut self) {
        self.sharing
            .active_tasks
            .store(999_999_999, Ordering::Release);
        self.sharing.condvar.notify_all();
    }
}

fn needs_thread(waiting: usize, queued: usize) -> bool {
    waiting <= queued
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Instant;

    #[test]
    fn needs_thread_counts_queued_jobs_as_reserved_workers() {
        assert!(needs_thread(0, 0));
        assert!(needs_thread(1, 1));
        assert!(needs_thread(4, 4));
        assert!(!needs_thread(2, 1));
    }

    #[test]
    fn a_burst_of_blocking_jobs_does_not_reserve_an_idle_worker_twice() {
        let pool = TaskPool::new();
        let deadline = Instant::now() + Duration::from_secs(2);
        while pool.sharing.waiting_tasks.load(Ordering::Acquire) != MIN_THREADS {
            assert!(Instant::now() < deadline, "workers never became idle");
            thread::yield_now();
        }

        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let (started_tx, started_rx) = mpsc::channel();
        let jobs = MIN_THREADS + 2;

        let mut queue = pool.sharing.todo.lock().unwrap();
        for _ in 0..jobs {
            let started_tx = started_tx.clone();
            let release = release.clone();
            pool.dispatch(
                &mut queue,
                Box::new(move || {
                    started_tx.send(()).unwrap();
                    let (released, condvar) = &*release;
                    let mut released = released.lock().unwrap();
                    while !*released {
                        released = condvar.wait(released).unwrap();
                    }
                }),
            );
        }
        drop(queue);

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut count = 0;
        while count < jobs {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if started_rx.recv_timeout(remaining).is_err() {
                break;
            }
            count += 1;
        }

        let (released, condvar) = &*release;
        *released.lock().unwrap() = true;
        condvar.notify_all();

        assert_eq!(count, MIN_THREADS + 2, "queued connections were starved");
    }
}
