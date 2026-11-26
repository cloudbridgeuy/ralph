use super::*;
use ralph_core::worker::{unattended, Approval, Unattended};

impl Pump {
    pub(super) fn server_request(
        &mut self,
        id: Value,
        method: &str,
        params: &Value,
    ) -> Result<(), Stop> {
        let stale = self.turns.is_stale_request(method, params);
        let Some(approval) = classify(method, params).filter(|_| !stale) else {
            return self.wire.send_line(&reject_server_request(&id, method));
        };
        if self.scheduled {
            return self.refuse_unattended(id, method, approval);
        }
        let auto = Some(self.mode)
            .filter(|mode| *mode == PermissionMode::Yolo)
            .and_then(|_| yolo_option(&approval));
        match auto {
            Some(option) => {
                let body = response_body(approval.kind, &option.option_id);
                self.wire.send_line(&answer_server_request(&id, &body))
            }
            None => {
                let content = BlockedContent {
                    request_id: id.as_str().map_or_else(|| id.to_string(), str::to_string),
                    ..approval.content
                };
                let reply = CodexReply {
                    id,
                    kind: approval.kind,
                };
                self.permissions.enqueue(reply, content);
                Ok(())
            }
        }
    }

    fn refuse_unattended(
        &mut self,
        id: Value,
        method: &str,
        approval: Approval,
    ) -> Result<(), Stop> {
        let verdict = unattended(&approval.content);
        self.permissions
            .report_unattended(&verdict, &approval.content);
        match verdict {
            Unattended::Allow(option) | Unattended::Deny(option) => {
                let body = response_body(approval.kind, &option.option_id);
                self.wire.send_line(&answer_server_request(&id, &body))
            }
            Unattended::Cancel => self.wire.send_line(&reject_server_request(&id, method)),
        }
    }
}
