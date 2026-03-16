# Conversation Loop Strategy

Human-driven conversation loop where the human writes in `$EDITOR`, a persona responds, and the loop repeats. Supports human-in-the-loop directives and agent-to-agent orchestration.

## Code Locations

| Component | File | Description |
|-----------|------|-------------|
| Transcript types (FC) | `crates/core/src/transcript.rs` | Speaker, TranscriptEntry, HumanResponse, LoopAction, CommentResponse |
| Transcript formatting (FC) | `crates/core/src/transcript.rs` | build_editor_content, parse_editor_response, check_exit, build_persona_prompt |
| Human classification (FC) | `crates/core/src/human.rs` | DirectiveTarget, classify_target, partition_directives |
| Comment extraction (FC) | `crates/core/src/directive.rs` | DirectiveVerb::Comment, extract_comments |
| Human I/O (IS) | `crates/ralph/src/human.rs` | open_editor_for_human, open_editor_for_ask, display_comment_and_wait |
| Strategy impl (IS) | `crates/ralph/src/strategy/conversation_loop.rs` | ConversationLoopStrategy, execute_conversation_loop, handle_directives |

## Flow

```
1. Initialize session
2. Open $EDITOR with transcript + separator
3. Parse human response below separator
4. If empty → exit
5. Build persona prompt with conversation history
6. Invoke persona via invoke_with_failure_recovery
7. Parse directives from response:
   a. extract_comments() → display human comments via terminal
   b. partition by target → human asks open editor, persona asks orchestrate
   c. If human responded → continue_session with aggregated response
8. Append both entries to transcript
9. Repeat from step 2
```

## Human-in-the-Loop Directives

Personas use the reserved target `"human"` to interact with the operator:

| Directive | Behavior |
|-----------|----------|
| `<ralph-ask to="human">` | Opens `$EDITOR` with question as context; response fed back to persona |
| `<ralph-comment to="human">` | Displays in terminal, soft-blocks until Enter or typed response |

Comments are separated from asks/handovers via `extract_comments()` before `validate_directive_set()`, so they coexist with other directive types.

## Editor Content Format

```
[You]
Human's first message

[storyteller]
Persona's response

--- Write your response below this line ---
```

The separator constant is `EDITOR_SEPARATOR` in `transcript.rs`. Content below the separator is extracted; empty/whitespace = abort.

## Transcript Accumulation

Each turn appends a `TranscriptEntry` (Speaker + content) to a `Vec<TranscriptEntry>`. The full transcript is:
- Shown in the editor (read-only above separator)
- Sent to the persona as `<conversation_history>` XML wrapper via `build_persona_prompt()`

## Fiction Writing Team

The bundled `fiction-loop` strategy demonstrates conversation-loop with four personas:

| Persona | Role | Mode |
|---------|------|------|
| storyteller | Primary writer, narrative driver | bypassPermissions |
| editor-agent | Structure, pacing, prose quality | plan (advisory) |
| worldbuilder | Setting, lore, consistency | plan (advisory) |
| critic | Tension, stakes, reader engagement | plan (advisory) |

Strategy TOML: `assets/strategies/fiction-loop.toml`
