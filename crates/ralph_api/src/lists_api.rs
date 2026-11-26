use crate::config_api::store_name;
use crate::route::{error, json_ok, ApiConfig, ApiRequest, ApiResponse};
use ralph_core::schedule_command::task_rows;
use ralph_core::schedule_request::SCHEDULE_DIR;
use ralph_core::watch_command::watch_rows;
use ralph_core::watch_store::WATCHES_DIR;
use ralph_core::workers_view::worker_state_rows;
use ralph_server::schedule_store::ScheduleStore;
use ralph_server::watch_store::WatchStore;
use ralph_server::worker_facts::worker_facts;
use serde_json::json;
use std::path::Path;

pub(crate) fn schedules_response(store: &Path) -> ApiResponse {
    if !store.is_dir() {
        return error(404, "store not found");
    }
    let tasks = ScheduleStore::new(store.join(SCHEDULE_DIR)).read_all();
    json_ok(json!({"store": store_name(store), "rows": task_rows(&tasks)}))
}

pub(crate) fn schedules(_request: &ApiRequest, config: &ApiConfig) -> ApiResponse {
    schedules_response(&config.store)
}

pub(crate) fn watches_response(store: &Path) -> ApiResponse {
    if !store.is_dir() {
        return error(404, "store not found");
    }
    let saved = WatchStore::new(store.join(WATCHES_DIR)).read_all();
    json_ok(json!({"store": store_name(store), "rows": watch_rows(&saved)}))
}

pub(crate) fn watches(_request: &ApiRequest, config: &ApiConfig) -> ApiResponse {
    watches_response(&config.store)
}

pub(crate) fn workers_response(store: &Path) -> ApiResponse {
    if !store.is_dir() {
        return error(404, "store not found");
    }
    let rows = worker_state_rows(&worker_facts(store));
    json_ok(json!({"store": store_name(store), "rows": rows}))
}

pub(crate) fn workers(_request: &ApiRequest, config: &ApiConfig) -> ApiResponse {
    workers_response(&config.store)
}
