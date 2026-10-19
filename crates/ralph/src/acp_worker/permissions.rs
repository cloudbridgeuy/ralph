use super::*;

struct PendingPermission {
    responder: Responder<RequestPermissionResponse>,
    content: BlockedContent,
}

fn selected_response(command: WorkerCommand) -> Option<RequestPermissionResponse> {
    match encode(command)? {
        AcpCall::PermissionResponse(option_id) => Some(RequestPermissionResponse::new(
            RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(option_id)),
        )),
        _ => None,
    }
}

pub(super) struct Permissions {
    events: Events,
    mode: CursorPermissions,
    worker_id: String,
    inbox: Inbox,
    pub(super) heartbeat: Arc<Heartbeat>,
    queue: Mutex<VecDeque<PendingPermission>>,
}

impl Permissions {
    pub(super) fn new(
        markers: &WorkerMarkers,
        mode: CursorPermissions,
        worker_id: String,
        inbox: Inbox,
        events: Events,
    ) -> Arc<Self> {
        Arc::new(Self {
            events,
            mode,
            worker_id,
            inbox,
            heartbeat: Heartbeat::new(markers.activity.clone(), markers.gone.clone()),
            queue: Mutex::new(VecDeque::new()),
        })
    }

    fn queue(&self) -> MutexGuard<'_, VecDeque<PendingPermission>> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn publish(&self, queue: &VecDeque<PendingPermission>) {
        let event = match queue.front() {
            Some(pending) => WorkerEvent::PermissionAsked(pending.content.clone()),
            None => WorkerEvent::PermissionSettled,
        };
        push_event(&self.events, event);
    }

    pub(super) fn push(
        &self,
        request: &RequestPermissionRequest,
        responder: Responder<RequestPermissionResponse>,
    ) -> bool {
        let value = serde_json::to_value(request).unwrap_or_default();
        let content = permission_content(&value);
        if let Some(option) = auto_allow_option(self.mode, &content) {
            let command = WorkerCommand::AnswerPermission {
                option_id: option.option_id.clone(),
            };
            if let Some(response) = selected_response(command) {
                responder.respond(response).ok();
            }
            return true;
        }
        let at = chrono::Utc::now().to_rfc3339();
        let report = permission_alert_report(&self.worker_id, &at, &content);
        let mut queue = self.queue();
        queue.push_back(PendingPermission { responder, content });
        self.publish(&queue);
        self.inbox.post(&report, 0).ok();
        false
    }

    pub(super) fn answer(&self, text: &str) -> Result<(), String> {
        let mut queue = self.queue();
        let front = queue
            .front()
            .ok_or("no permission request is pending for this worker")?;
        let option = match_option(&front.content.options, text)
            .ok_or_else(|| no_option_message(&front.content.options))?;
        let command = WorkerCommand::AnswerPermission {
            option_id: option.option_id.clone(),
        };
        let response = selected_response(command);
        if let (Some(pending), Some(response)) = (queue.pop_front(), response) {
            pending.responder.respond(response).ok();
        }
        self.publish(&queue);
        self.heartbeat.touch();
        Ok(())
    }

    pub(super) fn cancel_all(&self) {
        let drained: Vec<PendingPermission> = self.queue().drain(..).collect();
        for pending in drained {
            pending
                .responder
                .respond(RequestPermissionResponse::new(
                    RequestPermissionOutcome::Cancelled,
                ))
                .ok();
        }
        push_event(&self.events, WorkerEvent::PermissionSettled);
    }
}
