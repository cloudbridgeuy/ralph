# Orchestration

This document describes the multi-agent orchestration system that allows personas to collaborate by emitting structured directives in their output.

## Code Locations

| Component | File | Description |
|-----------|------|-------------|
| Directive types & parser | `crates/core/src/directive.rs` | `Directive`, `DirectiveVerb`, `ValidatedDirectiveSet`, parsing, validation |
| Orchestrator entry point | `crates/ralph/src/orchestrator/mod.rs` | `scan_for_directives()`, `orchestrate()`, `Budget`, `OrchestrationConfig` |
| Ask executor | `crates/ralph/src/orchestrator/ask.rs` | `execute_asks()`, `resolve()` — ask round-trip and sub-directive resolution |
| Conversation loop | `crates/ralph/src/orchestrator/conversation.rs` | `conversation_loop()`, `ConversationConfig` — back-and-forth between two personas |
| Parallel invocation | `crates/ralph/src/orchestrator/parallel.rs` | `parallel_invoke()` — concurrent target invocation via `std::thread::scope` |
| Display | `crates/ralph/src/orchestrator/display.rs` | ANSI-styled routing status and orchestration summary (FC-IS: `format_*` pure, `print_*` I/O) |
| Persona instructions | `personas/*.md` | Delegation hardening, directive triggers, team collaboration |

## Directive Format

Directives are XML-style tags with a `ralph-` prefix that personas emit in their output text:

```
<ralph-{verb} to="{target}">{payload}</ralph-{verb}>
```

- **verb** — The action type (see Synonym Verbs below)
- **target** — The persona name to route to (e.g., `"reviewer"`, `"developer"`)
- **payload** — Free-form text passed as the prompt to the target persona

Example:

```
<ralph-ask to="reviewer">Please review the error handling in src/main.rs for edge cases.</ralph-ask>
```

```
<ralph-handover to="developer">Implement the caching layer as described in the architecture doc.</ralph-handover>
```

## Synonym Verbs

The parser normalizes multiple verb synonyms to two canonical verbs:

| Canonical verb | Synonyms |
|---------------|----------|
| `ask` | `tell`, `say`, `respond`, `reply` |
| `handover` | `delegate`, `pass`, `transfer` |

Unknown verbs are silently skipped. Verbs are case-sensitive (lowercase only).

## Orchestration Modes

### Handover

The originator delegates work to one or more target personas and **stops**. Each target runs independently with the directive payload as its prompt. If a target emits sub-directives, they are resolved recursively.

Flow: `originator emits handover` -> `target runs` -> `(target may emit sub-directives)` -> `done`

### Ask

The originator requests input from one or more target personas and **continues** after receiving responses. Targets are invoked sequentially, their responses are aggregated into a single prompt, and the originator's session is continued with that aggregated prompt.

Flow: `originator emits asks` -> `targets run sequentially` -> `responses aggregated` -> `originator continues with aggregated prompt` -> `(originator may emit more directives)`

### Conversation

When a target responds to an ask directive by emitting an ask back to the originator, the orchestrator enters a conversation loop. The two personas exchange messages back and forth until one side finishes without directing the other, or the budget runs out.

Flow: `originator asks target` -> `target asks originator back` -> `originator responds` -> `(continues until one side finishes without a directive)` -> `done`

Each round of the conversation consumes one budget unit. The loop terminates when:

- One side produces output with no directive targeting the other — the final response text is returned.
- The invocation budget is exhausted — returns `BudgetExhausted` error.

Only ask directives trigger conversation loops. If a target emits a handover instead of an ask-back, the handover is executed normally (no loop).

## Choosing Ask vs. Handover

The two directive types serve different purposes:

| | Ask | Handover |
|---|---|---|
| **Intent** | "I need input to continue my work" | "This task belongs to someone else" |
| **Originator** | Continues after getting responses | Stops — work is done from their perspective |
| **Response** | Aggregated and fed back to originator | Not returned — target works independently |
| **Budget cost** | Targets + 1 continuation | Targets only |

**Use ask when:**
- You need a review or opinion before proceeding
- You want to gather information from multiple personas and synthesize it
- The originator should make the final decision

**Use handover when:**
- The work is complete from the originator's perspective
- A different persona should own the next step
- The task is self-contained and doesn't need a response back

## Validation Rules

A single persona output can contain multiple directives, but they must all be the same type:

- All asks — valid (`ValidatedDirectiveSet::Asks`)
- All handovers — valid (`ValidatedDirectiveSet::Handovers`)
- Mixed asks and handovers — invalid, directives are ignored with a warning

Malformed directives (missing closing tag, mismatched tags, missing `to` attribute, unknown verb) are silently skipped during parsing.

## Nested Directive Handling

The parser scans left-to-right for top-level directives only. Directive-like tags inside a payload are treated as opaque text, not parsed as separate directives.

This matters when a persona embeds directive syntax as an example or instruction inside a payload:

```
<ralph-ask to="architect">Please ask the developer a question.
Use a <ralph-ask to="developer"> directive to pose your question.</ralph-ask>
```

The parser produces **one** directive targeting `architect`. The inner `<ralph-ask to="developer">` is part of the payload text, not a second directive.

**How it works:** After finding an opening tag, the parser locates the first matching closing tag (`</ralph-{verb}>`), takes everything between as the payload, then resumes scanning **after** the closing tag. This means:

- Sequential top-level directives are parsed correctly
- Nested tags with their own closing tags cause the outer payload to end at the first `</ralph-{verb}>` match (acceptable truncation)
- The parser never produces spurious directives from payload content

## Budget System

The budget prevents runaway orchestration loops by capping the total number of sub-invocations per orchestration session.

- **Default limit**: 10 invocations (defined as `DEFAULT_BUDGET` in `orchestrator/mod.rs`)
- The budget is shared across all recursive directive chains via `Arc<AtomicUsize>`
- Each persona invocation (target or continuation) consumes one unit from the budget
- When exhausted, the orchestrator returns `OrchestrationError::BudgetExhausted`
- A summary is printed at the end showing how many invocations were used out of the total

Example budget consumption for an ask with two targets:

1. Target A invocation (-1)
2. Target B invocation (-1)
3. Originator continuation (-1)
4. Total: 3 of 10 used

## Display

Orchestration events are styled with ANSI escape codes to stand out from LLM output. The display module follows FC-IS: pure `format_*` functions return styled strings, `print_*` wrappers handle I/O.

### Routing Status

Printed before each directive invocation (parallel, conversation, ask continuation):

```
▶ pm → ask → architect                                    [9/10]
  "What are the technical responsibilities?"
```

- `▶` glyph in cyan signals an orchestration action
- Persona names in cyan, verb in plain text
- Budget fraction in dim brackets
- Payload preview on second line (truncated to 80 chars, dim)

### Orchestration Summary

Printed once at the end of the top-level `orchestrate()` call:

```
✓ Orchestration complete                                   [7/10]
```

- Green `✓` for success
- Budget fraction in dim brackets

### Spinner Context

When a persona is invoked via orchestration, the spinner shows who requested it:

```
⠋ Waiting for response... architect (for pm) (new-slug 1/?) | 3s
```

The `on_behalf_of` field on `InvocationConfig` is threaded through `invoke()` to `SpinnerSessionInfo`. Only shown when a persona is set (orchestration always sets persona).

### Conversation Loop Turns

The conversation loop prints a routing status line before each `continue_session()` call, making the back-and-forth structure visible:

```
▶ architect → ask → pm                                     [7/10]
  "What's the current command structure?"

[PM output...]

▶ pm → ask → architect                                     [6/10]
  "Here's the CLI structure: ..."

[Architect output...]
```

## Parallel Invocation

When a persona emits multiple directives of the same type, all targets are invoked concurrently using `std::thread::scope`. This applies to both ask and handover directives.

- Routing status lines are printed before threads spawn (to avoid interleaved output)
- Each thread consumes one budget unit independently
- Results are collected in directive order after all threads complete
- Sub-directives from each target are resolved sequentially after the parallel phase

## Session Continuation (Ask Round-Trip)

The ask round-trip relies on session continuation to feed aggregated responses back to the originator:

1. The originator runs and produces output containing ask directives
2. The orchestrator invokes each target persona as a fresh session
3. Each target's response text is collected (with sub-directive resolution if needed)
4. Responses are aggregated into a single prompt using `aggregate_responses()`:
   ```
   Response from reviewer:

   {reviewer's response}

   ---

   Response from tester:

   {tester's response}
   ```
5. The originator's session is continued via `continue_session()`, which:
   - Looks up the originator's session directory by slug
   - Counts existing iteration files to determine the next sequence number
   - Invokes the persona with conversation history (the `--continue` flag)
6. If the continuation output contains more directives, they are resolved recursively

## Sub-Directive Resolution

When a target persona itself emits directives (sub-directives), the orchestrator resolves them before returning the target's response:

- **No sub-directives**: the target's response text is returned directly
- **Sub-directives to other personas**: they are executed, and the target is continued with the sub-results
- **Sub-directive back to originator**: enters a conversation loop where the two personas exchange messages until one side finishes without directing the other (see Conversation mode above)

This allows multi-hop collaboration chains while preventing infinite loops between two personas.

## Persona Prompt Structure

Persona prompts are structured to make directive emission the path of least resistance. Without this structure, personas tend to use tools (Read, Grep, Glob) to do work outside their domain rather than delegating — even when the user explicitly requests delegation.

Each persona file (`personas/*.md`) follows this structure:

```
Identity paragraph (includes delegation as core behavior)
├── How you work (includes delegation behavior)
├── What you do
├── What you don't do (prescriptive — each "don't" paired with "Instead: <directive>")
├── Before you act (stop-and-check before using tools for investigation)
├── Directive triggers (non-negotiable rules mapping patterns to directive emission)
├── [persona-specific sections]
└── Team collaboration (directive syntax, when to delegate, writing good directives, budget)
```

### Key Sections

**"What you don't do"** — Prescriptive, not descriptive. Each item tells the persona what to do *instead*:

```markdown
- Analyze code structure, module boundaries, or technical responsibilities.
  Instead: `<ralph-ask to="architect">your question</ralph-ask>`
```

**"Before you act"** — Stop-and-check that interrupts the tool-use reflex:

```markdown
Before using Read, Grep, or Glob to investigate something, ask:
- Is this within MY domain?
- Or would another persona do this better?
```

**"Directive triggers"** — Non-negotiable rules for directive emission:

```markdown
- User says "ask the [persona]" → emit `<ralph-ask to="persona">`
- You need to understand code architecture → emit `<ralph-ask to="architect">`
```

### Adding New Personas

When creating a new persona, follow the same structure. The delegation hardening sections ("Before you act", "Directive triggers", prescriptive "What you don't do") are required — without them, the persona will default to doing everything itself.
