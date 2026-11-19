# Chat decay check

`ralph chat history decay [--max-anchors <N>] [--store <DIR>]` tests one question: does repeated hierarchical compression make old decisions harder to find? It reads `log.jsonl`, `tree.jsonl` and `tags.jsonl` from the store. It makes no model call, takes no turn and costs nothing. A missing `tags.jsonl` reads as no tags. Put `--store` and `--thread` before `history` or after `decay`.

## Run it

```bash
ralph chat history decay
ralph chat history decay --max-anchors 50 --store ~/.ralph/chat
```

## Anchors

An anchor is an old message the check tries to find again. There are two groups. The report keeps them apart in every section.

- tagged: entries whose latest tag row says `rule` or `decision`. Failed and skipped rows do not count.
- top-up: untagged user entries that hold a ticket id (like `guz-1637`) or an identifier (like `split_view` or `a::b`).

`--max-anchors` (default 200) caps the total. Tagged anchors come first.

## Key tokens

Each anchor entry gives key tokens, found with the search tokenizer. A token falls in the first class that fits.

1. ticket: letters, a dash, digits.
2. number: only digits.
3. identifier: a compound word (it has parts) or a word with `_`.
4. rare: a plain word of four or more letters that is in at most three log entries.

Other words are ignored. A `#id` reference is not a token. It is a pointer.

## What each number means

- Header: `anchors N (tagged X, top-up Y) uncompressed U`. U counts anchors whose cover node is a single entry. They have no summary, so the other numbers leave them out.
- Survival: for each summary above the anchor (its aligned blocks with a summary), the share of the anchor's key tokens that the summary still holds. Rows are depths (block size is 2 to the power of depth). Columns are token classes.
- Search rank: the query is the key tokens that survive in the summary the chat view shows for the anchor (the cover node). The report gives the 1-based rank of the anchor in `search` results, in buckets 1, 2-5, 6-20, >20 and miss.
- unreachable: no key token survives in the cover summary, so no query exists.
- Recall found (newest 50): the share of anchors that `recall` lists among its newest 50 matching lines (the cap of the recall tool). A match older than the 50 newest counts as a miss, so this is a floor. The pattern is the surviving tokens joined by `|`.
- Pointer kept: the share of anchors whose `#id` appears in any summary above them. `#12` does not match `#123`.

A falling survival or rank at higher depth means compression loses old facts.

## Era split is approximate

Each section also splits by era. The era of a number is the date of the last entry in its summary block.

- before 2026-11-12
- 2026-11-12
- from 2026-11-13

The edges come from the nap prompt history in `nap.rs`: GUZ-1599 and GUZ-1615 on 2026-11-12, GUZ-1624 on 2026-11-13. The split is approximate: a summary is written after its entries, so its real prompt version can be later than its date.

## Code

The pure `build_decay` and `render_decay` live in `crates/core/src/chat_memory/decay/`. The shell is `crates/ralph/src/chat/session/decay.rs`.
