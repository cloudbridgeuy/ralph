use crate::chat::fork::{open_fork, Fork};
use crate::chat::memory::{Answer, Reach, Store};
use crate::history_log;
use ralph_core::chat_memory::{resolve_scope, CallContext, CallSource, MemoryCall};
use ralph_core::chat_thread::{fork_check, Scope, ThreadName, THREADS_DIR, THREAD_FILE};
use std::path::{Path, PathBuf};
use std::time::Instant;

pub struct Memory {
    own: Store,
    fork: Option<Fork>,
}

impl Memory {
    pub fn open(dir: &Path) -> Result<Memory, String> {
        Ok(Memory {
            own: Store::new(dir.to_path_buf()),
            fork: open_fork(dir)?,
        })
    }

    pub fn call(&self, call: &MemoryCall, source: CallSource) -> Result<Answer, String> {
        let started = Instant::now();
        let result = self.dispatch(call);
        let resolved = MemoryCall {
            scope: resolve_scope(&call.scope, self.fork.as_ref().map(|fork| &fork.name)),
            tool: call.tool.clone(),
        };
        history_log::record_call(
            self.dir(),
            &resolved,
            &result.clone().map(|answer| answer.text),
            CallContext {
                source,
                took: started.elapsed(),
                metrics: result
                    .as_ref()
                    .map(|answer| answer.facts.clone())
                    .unwrap_or_default(),
            },
        );
        result
    }

    fn dispatch(&self, call: &MemoryCall) -> Result<Answer, String> {
        let own = |label| self.own.call(&call.tool, Reach { limit: None, label });
        match (&call.scope, &self.fork) {
            (Scope::Own | Scope::Main, None) => own(None),
            (Scope::Own, Some(fork)) => own(Some(&fork.name)),
            (Scope::Thread(name), Some(fork)) if *name == fork.name => own(Some(name)),
            (Scope::Main, Some(fork)) => {
                fork_check(&call.tool, fork.meta.base)?;
                let reach = Reach {
                    limit: Some(fork.meta.base),
                    label: None,
                };
                fork.main.call(&call.tool, reach)
            }
            (Scope::Thread(name), fork) => {
                let main = fork.as_ref().map_or(&self.own, |fork| &fork.main);
                let dir = thread_dir(main.dir(), name);
                if !dir.join(THREAD_FILE).is_file() {
                    return Err(format!("no thread {name}; `ralph thread ls` lists them."));
                }
                let reach = Reach {
                    limit: None,
                    label: Some(name),
                };
                Store::new(dir).call(&call.tool, reach)
            }
        }
    }

    pub fn dir(&self) -> &Path {
        self.own.dir()
    }
}

pub fn thread_dir(main: &Path, name: &ThreadName) -> PathBuf {
    main.join(THREADS_DIR).join(name.as_str())
}

#[cfg(test)]
mod history_tests;
