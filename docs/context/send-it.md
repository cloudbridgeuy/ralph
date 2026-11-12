# send-it Guide

`/send-it` runs an approved plan with no input from you. You give instructions. The model writes them as a plan of numbered steps, and each step has an exit criterion. You read the plan and approve it one time. Then the chat runs the steps until the plan is done or the run stops. This guide shows the tasks. For the full behaviour and the exact notices, see [ralph chat: /send-it](ralph-chat.md#send-it).

It describes the behaviour of the current release.

## Propose a plan

Type `/send-it <instructions>` in the chat. This sends one turn to the model. The model writes the plan and does not start the work. ralph stores the plan as `<store>/send-it/run.toml` and shows it with its hash. The plan is not approved yet.

- If the turn stores no plan, ralph tells you. Ask again with clearer instructions.
- A step with no exit criterion is refused. The model must give each step a check that ralph can measure.
- A new proposal replaces a plan that is not approved.
- While an approved plan is active, ralph refuses a new plan. Stop it with `/send-it off` first.
- A bare `/send-it` shows the usage and the default caps.
- `/send-itx` is a normal message.

## The plan file

The plan is a TOML file. You can edit it with `/send-it edit`. A plan has a goal, one or more steps, the allowed actions, grants and caps:

```toml
goal = "ship GUZ-1"
allowed = [{ kind = "spawn", harness = "codex" }, { kind = "reply" }, { kind = "close" }]
grants = []

[[steps]]
do = "spawn a codex worker for GUZ-1"
criterion = { kind = "worker_done", worker = "GUZ-1" }

[[steps]]
do = "run the checks"
criterion = { kind = "check_passes", command = "cargo xtask lint" }

[caps]
turns = 15
usd = 5.0
judge_rounds = 2
```

After you approve, the file also has an `approved` line with the hash. Do not write it yourself. Use `/send-it go`.

## Review and approve

- `/send-it status` shows the stored plan, its hash and its approval.
- `/send-it edit` opens the plan in the chat editor (`[chat] editor`, `$VISUAL`, `$EDITOR`). After you save, ralph removes the approval and shows the plan again. If the file does not parse, ralph shows the error. Run `/send-it edit` again to fix it.
- `/send-it go` approves the plan and starts the run.
- `/send-it off` deletes the plan.

None of these starts a model turn, except the run that `/send-it go` starts.

`/send-it go` approves only the plan whose hash the last `/send-it` output showed. Type it at the prompt. ralph refuses a `/send-it go` that you queue while a turn runs.

The hash follows the content of the plan, not its layout. Any change to the content clears the approval, also an edit made outside ralph. A started run uses up the hash that ralph showed. To run the plan again, show it with `/send-it status`, then type `/send-it go`.

## Exit criteria

Each step has one criterion. The run goes to the next step only when the criterion is met.

- `worker_done { worker }`: the worker reports `done`. Any other report status from that worker stops the run.
- `check_passes { command }` and `watch_passes { command }`: ralph runs the command as a watch. The step is met when the command exits 0. Any other result stops the run.
- `judge { question }`: the model answers the question with "met" or "not met" and one line of evidence. See [Judge steps](#judge-steps).

A question or a blocked report from any worker stops the run, unless the plan allows `reply` and the model replied to that worker in the turn that read the report. A report that arrives during a turn and is not read yet still stops it. If the plan does not allow `reply`, ralph refuses the model's reply, so the worker gets no answer, and the run stops.

## Judge steps

A judge step uses the chat model to decide if the step is met. The model must give a yes or no answer and one line of evidence of at most 300 characters.

If the answer is "not met", or the model gives no answer, that is a failed round. ralph sends the step again with the evidence. When the judge rounds are used up, the run stops. The default is 2 rounds. Set `judge_rounds` in `[caps]` to change it.

## Caps and what counts

Each run has a turn cap and a cost cap. When the run reaches a cap, it stops.

- The plan sets the caps in `[caps]`. A plan with no caps uses `[chat] send_it_max_turns` (default 15) and `[chat] send_it_max_usd` (default 5.0). See [configuration](../configuration.md).
- A grant (`turn_cap` or `cost_cap`) replaces that cap. The plan shows grants in a separate "You pre-approve" block, so you see them before you approve.
- These turns count toward the turn cap: step turns, judge turns, and report turns that settle nothing.
- The turn that reads a worker or watch report and settles its step does not count.
- The cost of every run turn counts toward the cost cap, also the free report turns. A turn costs its own spend, not the running total of the claude process, so the run cost equals the sum of the status bar deltas of the run turns. Keepalive pings, naps and workers do not count.

Usage rows of run turns have the trigger `send_it`.

## Allowed actions

The plan lists the actions the run can do. Read this list before you approve.

- ralph enforces some actions: spawn a worker (only with the listed harness), reply to workers, close workers (not with abandon or force), and start watches. A run can never create a schedule.
- Other actions (merge, push, open PRs, write to Linear, paid runs) depend only on the instructions. ralph does not block them.

If the model uses a tool that the plan does not allow, the tool is refused and the run stops. For the full list, see [ralph chat: /send-it](ralph-chat.md#send-it).

## During a run

Before each step turn, ralph shows the step, the turns used and the cost against the caps. When a step is met, ralph tells you.

While a run continues, ralph shows no suggested reply and does not hold on its own.

To take over, type a message, or leave the run's hold with Ctrl-Q or Ctrl-D. The run stops and your message runs as usual. A queued `/send-it off` stops the run and deletes the plan.

## Stops

A run stops when:

- it reaches a cap,
- a step misses its criterion, or its check fails,
- a worker asks a question or is blocked and the model did not reply to it under an allowed `reply`,
- the last judge round is not met,
- the plan changes or loses its approval,
- a turn ends with no reply,
- a schedule waits for your approval,
- the model uses a tool that the plan does not allow,
- you take over.

ralph shows a notice with the reason and the step that stopped, and tells you how to resume. It also posts an alert to the inbox. When all steps are met, ralph shows how many turns and how much cost the run used. This cost is the cost of the run turns only, with no pings, naps or workers. Worker cost shows in `ralph usage`, not in the run cost.

The model sees each stop and done notice, and the result of each `/send-it off`, `go`, `edit` and `save`, before its next message (see [Local command records](ralph-chat.md#local-command-records)). `/send-it status` is not recorded.

After a stop or a finished run, the plan stays in `run.toml` without its approval, until you type `/send-it off`. You can save it as a template or propose a new plan.

To resume after a stop, fix the cause (for example, answer the worker), show the plan with `/send-it status`, then type `/send-it go`. The status line says `stopped at step N/M`, and the run starts again at step N, with fresh caps. Steps before N do not run again. ralph first checks if step N is already met. For a worker step, a `done` report from the worker moves the run to the next step; a worker that is running, or that last reported anything else, makes the run wait for its report and does not send the step again; a worker ralph does not know gets the step again. A check or watch step runs its command first: exit 0 moves on, any other result sends the step again. A judge step is judged first: not met sends the step again, and this first judgement does not count as a judge round. A met step prints `send-it: step N/M met`. If you edit the plan, its hash changes, and `/send-it go` starts at step 1. After a finished run, `/send-it go` starts at step 1.

For the exact notices, see [ralph chat: /send-it](ralph-chat.md#send-it).

## Quit during a run

The run lives only in the chat process. If you quit the chat, the run ends. The next `/send-it go` starts at the step of the last stop of this plan, or at step 1 if it did not stop.

The plan stays approved after a quit. ralph refuses new plans until you run `/send-it off`.

## Templates

Templates are optional. Use them to run the same plan for a different ticket.

- `/send-it save <name> [<ticket>]` saves the stored plan, approved or not, as a template. Each whole `<ticket>` in the plan becomes `{ticket}`. With no ticket, or a ticket not in the plan, ralph saves the plan as it is and tells you. Saving over a name replaces that template.
- `/send-it templates` lists the saved templates.
- `/send-it <ticket> <name>` fills the template with the ticket and stores it as the plan, not approved. ralph shows it like `/send-it status`. Type `/send-it go` to approve it.

Names and tickets use letters, digits and dashes. Only two words where the second is a saved template name use a template. Other text is instructions for a new plan.

Templates are in the chat's own store. Threads do not share them.

## Refusals

ralph refuses:

- a new plan, or a template fill, while an approved plan is active,
- a step with no exit criterion,
- a `/send-it go` for a plan whose hash was not shown last, or a queued `/send-it go`,
- a tool that the approved plan does not allow, during a run. This includes a reply to a worker question in the turn that reads the report.

For the exact messages, see [ralph chat: /send-it](ralph-chat.md#send-it).
