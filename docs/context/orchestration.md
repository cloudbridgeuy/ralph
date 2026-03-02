# Orchestration

This document describes the multi-agent orchestration system that allows personas to collaborate by emitting structured directives in their output.

## Code Locations

| Component | File | Description |
|-----------|------|-------------|
| Directive types & parser | `crates/core/src/directive.rs` | `Directive`, `DirectiveVerb`, `ValidatedDirectiveSet`, parsing, validation |
| Orchestrator entry point | `crates/ralph/src/orchestrator/mod.rs` | `scan_for_directives()`, `orchestrate()`, `Budget`, `OrchestrationConfig` |
| Ask executor | `crates/ralph/src/orchestrator/ask.rs` | `execute_asks()`, `resolve()` — ask round-trip and sub-directive resolution |
| Display | `crates/ralph/src/orchestrator/display.rs` | Routing status lines and orchestration summary |
| Persona instructions | `personas/*.md` | Team collaboration section with directive syntax |

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

### Conversation (not yet implemented)

When a target emits an ask directive back to the originator, this creates a conversation loop. This mode is not yet implemented and will return an error.

## Validation Rules

A single persona output can contain multiple directives, but they must all be the same type:

- All asks — valid (`ValidatedDirectiveSet::Asks`)
- All handovers — valid (`ValidatedDirectiveSet::Handovers`)
- Mixed asks and handovers — invalid, directives are ignored with a warning

Malformed directives (missing closing tag, mismatched tags, missing `to` attribute, unknown verb) are silently skipped during parsing.

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
- **Sub-directive back to originator**: returns an error (conversation loops not yet implemented)

This allows multi-hop collaboration chains while preventing infinite loops between two personas.
