use crate::cursor_owner::write_file;
use crate::hook_bridge::{deliver, HookQueue, HookSocket};
use crate::permission_queue::{host_process_token, PermissionQueue};
use crate::wire::store::{current_uid, ensure_store_root};
use crate::worker_events::Events;
use crate::worker_mcp::Inbox;
use ralph_core::chat_socket::{check_socket_path, runtime_root, StoreKey};
use ralph_core::worker::{
    add_bridge_hook, bridge_hook_command, hook_socket_name, remove_bridge_hook, PermissionAnswer,
    SpawnRequest, CURSOR_HOOKS_FILE,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

static SOCKET_SERIAL: AtomicU64 = AtomicU64::new(0);

pub(super) struct PrintHooks {
    queue: Arc<HookQueue>,
    hooks_file: PathBuf,
    created: bool,
    _socket: HookSocket,
}

impl PrintHooks {
    pub(super) fn start(
        id: &str,
        request: &SpawnRequest,
        store: &Path,
        events: &Events,
    ) -> Result<Self, String> {
        let root = runtime_root(current_uid());
        ensure_store_root(&root).map_err(|e| format!("{}: {e}", root.display()))?;
        let serial = SOCKET_SERIAL.fetch_add(1, Ordering::SeqCst);
        let path = root.join(hook_socket_name(
            &StoreKey::of(store),
            process::id(),
            serial,
        ));
        check_socket_path(&root, &path)?;
        let socket = HookSocket::bind(&path).map_err(|e| {
            format!(
                "could not bind the cursor hook socket {}: {e}",
                path.display()
            )
        })?;
        let queue = Arc::new(PermissionQueue::new(
            host_process_token(),
            id.to_string(),
            Inbox::new(request.inbox.clone()),
            Arc::clone(events),
        ));
        socket
            .serve(Arc::clone(&queue))
            .map_err(|e| e.to_string())?;
        let hooks_file = request.worktree.join(CURSOR_HOOKS_FILE);
        let existing = fs::read_to_string(&hooks_file).ok();
        let command = bridge_hook_command(&request.exe, socket.path());
        write_file(&hooks_file, &add_bridge_hook(existing.as_deref(), &command))
            .map_err(|e| format!("could not write {}: {e}", hooks_file.display()))?;
        Ok(Self {
            queue,
            hooks_file,
            created: existing.is_none(),
            _socket: socket,
        })
    }

    pub(super) fn answer(&self, answer: &PermissionAnswer) -> Result<(), String> {
        let (reply, option) = self
            .queue
            .take_answer(&answer.request_id, &answer.option_id)?;
        deliver(reply, option.option_id, &answer.request_id)
    }
}

impl Drop for PrintHooks {
    fn drop(&mut self) {
        drop(self.queue.drain());
        if self.created {
            fs::remove_file(&self.hooks_file).ok();
        } else if let Ok(text) = fs::read_to_string(&self.hooks_file) {
            write_file(&self.hooks_file, &remove_bridge_hook(&text)).ok();
        }
    }
}
