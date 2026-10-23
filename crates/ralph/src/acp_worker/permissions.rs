use super::*;
use crate::permission_queue::PermissionQueue;

type Reply = Responder<RequestPermissionResponse>;

fn selected_response(command: WorkerCommand) -> Option<RequestPermissionResponse> {
    match encode(command)? {
        AcpCall::PermissionResponse(option_id) => Some(RequestPermissionResponse::new(
            RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(option_id)),
        )),
        _ => None,
    }
}

fn respond_selected(responder: Reply, option_id: String) {
    if let Some(response) = selected_response(WorkerCommand::AnswerPermission { option_id }) {
        responder.respond(response).ok();
    }
}

pub(super) struct Permissions {
    mode: PermissionMode,
    queue: PermissionQueue<Reply>,
    pub(super) heartbeat: Arc<Heartbeat>,
}

impl Permissions {
    pub(super) fn new(
        markers: &WorkerMarkers,
        mode: PermissionMode,
        worker_id: String,
        inbox: Inbox,
        events: Events,
    ) -> Arc<Self> {
        Arc::new(Self {
            mode,
            queue: PermissionQueue::new(worker_id, inbox, events),
            heartbeat: Heartbeat::new(markers.activity.clone(), markers.gone.clone()),
        })
    }

    pub(super) fn push(&self, request: &RequestPermissionRequest, responder: Reply) -> bool {
        let value = serde_json::to_value(request).unwrap_or_default();
        let content = permission_content(&value);
        if let Some(option) = auto_allow_option(self.mode, &content) {
            respond_selected(responder, option.option_id.clone());
            return true;
        }
        self.queue.enqueue(responder, content);
        false
    }

    pub(super) fn answer(&self, text: &str) -> Result<(), String> {
        let (responder, option) = self.queue.take_answer(text)?;
        respond_selected(responder, option.option_id);
        self.heartbeat.touch();
        Ok(())
    }

    pub(super) fn cancel_all(&self) {
        for responder in self.queue.drain() {
            responder
                .respond(RequestPermissionResponse::new(
                    RequestPermissionOutcome::Cancelled,
                ))
                .ok();
        }
    }
}
