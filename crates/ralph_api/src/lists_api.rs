use crate::config_api::store_name;
use crate::route::{error, json_ok, ApiConfig, ApiRequest, ApiResponse};
use ralph_core::schedule_command::task_rows;
use ralph_core::schedule_request::SCHEDULE_DIR;
use ralph_server::schedule_store::ScheduleStore;
use serde_json::json;
use std::path::Path;

pub(crate) fn schedules_response(store: &Path) -> ApiResponse {
    if !store.is_dir() {
        return error(404, "store not found");
    }
    let tasks = ScheduleStore::new(store.join(SCHEDULE_DIR)).load_all();
    json_ok(json!({"store": store_name(store), "rows": task_rows(&tasks)}))
}

pub(crate) fn schedules(_request: &ApiRequest, config: &ApiConfig) -> ApiResponse {
    schedules_response(&config.store)
}
