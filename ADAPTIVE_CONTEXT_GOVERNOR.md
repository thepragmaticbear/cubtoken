# cubtoken Adaptive Context Governor — Go-forward plan

Status: product and implementation plan. The Adaptive Context Governor is the core product; optional output-token discipline follows on the same task-level measurement foundation.

## 1. Product outcome

cubtoken should become the first native closed-loop context optimizer for coding agents: compress, observe what the agent retrieves afterward, measure context regret, and safely adapt.

Payload compression is not the outcome. A tool result shrinking from 10,000 tokens to 2,000 tokens is useful only if the agent does not spend the difference on additional Reads, searches, reasoning, retries, or verbose output.

The runtime and user-facing north star is **Net Context Saved**:

```text
net_context_saved =
    gross_context_saved
    - attributable_recovery_context
```

The end-to-end validation metric is **Net Task Tokens**:

```text
task_tokens = all model input tokens + all model output tokens

net_task_token_savings =
    baseline task_tokens
    - optimized task_tokens
```

Task success is a gate, not a term that can be traded away. A failed or materially degraded task never counts as a token win.

cubtoken has one core optimization surface and one optional companion:

| Surface | Control | Feedback |
| --- | --- | --- |
| Core: model input/context | Compress native tool results before Claude sees them. | Did Claude retrieve the hidden context later? |
| Companion: model output | Optionally shape Claude toward concise responses before generation. | Did output fall without reducing task success or causing corrective turns? |

The input/context loop ships first because cubtoken already controls Read results and records targeted follow-up Reads. The output loop is built on the same task-level benchmark, but it remains opt-in until its quality and recovery signals are proven.

### Context regret

**Context regret** is the internal name for context that must be reintroduced because an earlier compression decision hid information the agent later needed.

For the first implementation:

```text
context_regret(decision) =
    high-confidence attributable recovery result tokens

net_context_saved =
    gross_context_saved - context_regret
```

User-facing reports call this **recovery overhead**. “Regret” remains an engineering and policy term because recovery can be correct and intentional.

Net Context Saved is the local control-loop and CLI headline. Net Task Tokens answers the broader benchmark question: did cubtoken reduce total consumption for a successful task?

The feedback loop is:

```text
Read result
  -> deterministic compression decision
  -> later tool batch
  -> attributable recovery, or no recovery
  -> local bucket outcome
  -> future compression may back off
```

The first adaptive release is deliberately conservative:

- Read only
- local, deterministic, and inspectable
- no model calls, embeddings, proxy, or hosted telemetry
- adaptive behavior may only reduce configured compression
- automatic backoff is one-way until the user resets it

### Product positioning

The differentiator is not “higher compression.” It is:

> cubtoken measures whether Claude needed context back after compression, then safely backs off when compression causes context regret.

Keep the product narrow: a tiny, deterministic, inspectable Rust binary using native Claude Code hooks—not a proxy, memory layer, retrieval platform, or general context-middleware competitor.

Treat “the first closed-loop token optimizer for coding agents” as target positioning. Validate the “first” claim against current products and published benchmarks before using it publicly.

## 2. Non-goals

Do not include these in the first adaptive context release:

- conversation-history compression or prompt-cache manipulation
- adaptive Grep, Glob, Bash, or output-policy learning
- generated summaries or learned source-specific policy
- graduated `balanced` and `expanded` skeleton profiles
- arbitrary user-tunable learning thresholds

Do not truncate or summarize an assistant response after generation. That cannot reduce billed output tokens and can remove information the user needs.

Output-token control remains in this roadmap as a separate, opt-in governor. It must shape generation up front and use an independent experiment so its effect is measurable.

## 3. Existing invariants

The governor must preserve the current Read behavior:

- targeted Reads pass through unchanged
- excluded and session-edited files pass through unchanged
- files at or below the configured threshold pass through unchanged
- compression uses the existing tree-sitter or head/tail representation
- replacement occurs only when the result is at least 30% smaller

The governor selects whether compression is permitted. It does not create a new representation.

All hook and state failures remain fail-open: return the original tool output, emit no protocol noise, and never block Claude.

## 4. Causal event model

User-turn boundaries alone are insufficient because Claude can issue parallel tool calls and their `PostToolUse` hooks can run concurrently. Attribution must include tool-batch boundaries.

Register the existing `ctk hook` executable for these events:

| Event | Action |
| --- | --- |
| `SessionStart` | Optionally inject one configured output profile instruction. |
| `UserPromptSubmit` | Start a turn; implicitly close any unfinished prior turn. |
| `PostToolUse` | Apply current compression and append tool events. |
| `PostToolBatch` | Append one causal batch boundary after all calls in the batch. |
| `Stop` | Record estimated visible output tokens, close the turn, and refresh adaptive state. |
| `StopFailure` | Close the failed turn without treating it as a successful task outcome. |
| `SessionEnd` | Best-effort final close and refresh. |

`Stop` is not guaranteed after interruption or API failure. The next `UserPromptSubmit` and `SessionEnd` are required fallbacks.

Only `PostToolUse` may emit a replacement tool result. `SessionStart` may emit the configured output instruction as `additionalContext`. Other bookkeeping events print nothing.

### Protocol parsing

Replace the single required `HookPayload` shape with a tolerant envelope:

```text
HookEnvelope
  common: session_id, cwd, transcript_path?, permission_mode?, effort?
  event:
    SessionStart
    PostToolUse { tool_name, tool_input, tool_response, tool_use_id?, duration_ms? }
    UserPromptSubmit
    PostToolBatch
    Stop { last_assistant_message? }
    StopFailure
    SessionEnd
```

For old fixtures without `hook_event_name`, infer `PostToolUse` only when `tool_name` is present. Unknown events pass through.

## 5. Decision metadata

Extend the existing `save` ledger record rather than introducing a second decision format.

```rust
CompressionDecision {
    decision_id,
    turn,
    batch,
    sequence,
    recorded_at_ms,
    file_identity,
    content_fingerprint,
    language,
    strategy,
    profile,
    tokens_in,
    tokens_out,
    elided_ranges,
}
```

Rules:

- Use `tool_use_id` as `decision_id`; fall back to `session_id:sequence`.
- Prefer the response's `/file/filePath`; otherwise resolve the input path against `cwd`.
- Canonicalize when possible and retain a lexical normalized fallback.
- Store a versioned deterministic 64-bit fingerprint, not source content.
- Use Unix milliseconds only for cross-session recency; batch numbers establish causality.

Paths and fingerprints remain in session ledgers. The aggregate adaptive file stores neither.

### Elided ranges

Extend `ctk-sitter::Skeleton` with structured `elided_ranges`. Record each range in `Builder::emit_gap` at the same point that renders its `[Lx-Ly]` marker. Do not recover ranges later by parsing display text.

Keep `shown_lines` as an invariant oracle: every source line must be either shown verbatim or covered by an ordered, non-overlapping elided range.

The head/tail fallback returns the same range metadata from its fixed visible regions. `SkeletonPolicy` and graduated body profiles remain deferred.

## 6. Recovery attribution

### Targeted Read

A targeted Read is high-confidence recovery only when all conditions hold:

1. The matched decision is from the same session and a strictly earlier tool batch.
2. Normalized file identities match and no Edit, Write, or NotebookEdit invalidated the decision.
3. The current on-disk fingerprint matches the decision fingerprint.
4. The returned `startLine..startLine+numLines` overlaps an elided range.
5. The Read occurs in the same user turn.

If the fingerprint cannot be verified, downgrade the event. If the range does not overlap, classify it as a normal targeted Read.

The next turn with valid overlap is medium confidence. Later or causally ambiguous reads are low confidence. Only high-confidence events train the first policy.

### Full-file repeat Read

A full Read is a high-confidence repeat only when it is in the same turn, in a strictly later batch, and has matching identity and fingerprint with no intervening edit.

The behavior is:

```text
first full Read  -> normal compact compression
later-batch repeat full Read -> pass through once and record recovery
same-batch full Read -> independent request; do not attribute
```

Use the existing `refetch` record with optional `decision_id`, `kind`, `confidence`, `turn`, `batch`, and token fields. `kind` is `targeted` or `full_repeat`. Old refetch records remain readable but do not train adaptive policy.

## 7. Metrics

For each decision:

```text
gross_saved = tokens_in - tokens_out
recovery_cost = sum(high-confidence recovery result tokens)
net_saved = gross_saved - recovery_cost
```

Report both:

- recovery overhead: `recovery_cost / gross_saved`
- recovery event rate: recovered decisions / compression decisions
- gross and attributed net context saved
- tool time added by recovery Reads
- confidence breakdown for recorded recovery candidates

Keep the current broad refetch calculation labeled as legacy until old ledgers age out. Do not call recovery tokens “waste.”

### Runtime measurement versus task measurement

The local hook ledger can estimate:

- tool-result tokens entering context
- gross context saved and attributable context regret
- visible assistant-response tokens from `Stop.last_assistant_message`
- recovery tool calls, tokens, and elapsed tool time

It cannot authoritatively reconstruct all model input and output billing from supported hook fields. `ctk stats` must label its values as estimates and must not present Net Context Saved as total task-token savings.

The live benchmark harness measures Net Task Tokens using authoritative usage fields from the supported Claude CLI or SDK result. Transcript parsing may enrich diagnostics, but runtime compression and the primary benchmark must not depend on undocumented transcript internals.

## 8. Adaptive policy v1

Bucket decisions by:

```text
language x input-size band x compression strategy
```

Initial size bands are `2k-4k`, `4k-8k`, `8k-16k`, `16k-32k`, and `32k+`. Unknown languages use `other|...|head_tail`.

Modes:

```text
off      current static behavior
observe  compute and report recommendations without changing behavior
safe     apply only less-aggressive recommendations
```

Candidate safe recommendations are `1x`, `2x`, `4x`, and `disabled`, applied to the configured threshold. Exclusions, edit protection, targeted-read pass-through, and the 30% savings floor always win.

Calibrate exact thresholds in observe mode. The starting policy may use:

```text
fewer than 8 decisions       -> 1x
recovery overhead > 20%      -> 2x
recovery overhead > 35%      -> 4x
net savings <= 0             -> disabled
recovery event rate > 40%    -> at least 2x
```

V1 does not automatically lower a multiplier. Once a bucket backs off, it remains there until `ctk adaptive reset`. This avoids pretending a disabled bucket can collect positive evidence.

A later release may add sparse probation reads. That requires a separate design and benchmark.

## 9. Optional output discipline

Output discipline is a companion feature, not cubtoken's core identity. It reduces model output tokens by shaping the response before generation and never rewrites a completed response.

Initial config:

```toml
[output]
mode = "default" # default | concise
```

`concise` installs one short instruction through `SessionStart`:

```text
Do not narrate routine tool use or restate tool output.
After successful work, report only material changes, failures,
and required next actions unless the user asks for more detail.
```

Requirements:

- opt-in only; `default` emits no instruction
- record token counts, never assistant text, in cubtoken state
- benchmark separately against the same tasks and success rubric
- count corrective follow-up turns in Net Task Tokens
- never become more concise than the user-configured baseline automatically

V1 output control is a fixed profile, not an adaptive learner. Adaptive output policy requires a reliable causal signal for “the response was too short”; keyword guessing over user prompts is not sufficient.

## 10. Adaptive state and reset semantics

Use:

```text
.cubtoken/session-<id>.jsonl  raw backward-compatible events
.cubtoken/adaptive-v1.json    bounded policy state and durable reset epoch
```

The adaptive file contains:

```json
{
  "schema": 1,
  "policy_version": 1,
  "reset_at_ms": 0,
  "buckets": {}
}
```

`ctk adaptive reset` atomically replaces this file with empty buckets and `reset_at_ms = now`. Historical ledgers remain available to `ctk stats`, but outcomes older than the reset epoch never rebuild learned policy.

The bucket aggregates are reconstructible. The reset epoch is durable control state, so the adaptive file is not described as a pure cache.

If the file is missing or corrupt:

- use static configured behavior immediately
- initialize empty adaptive state at the current time
- do not resurrect old learning automatically

### Aggregation and concurrency

At turn finalization, rebuild bucket aggregates deterministically from eligible ledger outcomes newer than `reset_at_ms`, retain the latest 32 outcomes per bucket, and atomically replace the snapshot.

Concurrent snapshot writers may temporarily omit the newest event; a later deterministic rebuild converges. No compression correctness depends on the snapshot.

Per-session ledger replay and append must be serialized under a file lock because parallel `PostToolUse` processes share the same ledger. If lock acquisition fails, skip recording and preserve normal fail-open compression behavior.

Do not add SQLite or a session snapshot until benchmarks demonstrate the need.

## 11. Read decision order

Every full Read follows this order:

1. Apply hard gates: enabled, full Read, exclusion, and edit protection.
2. Check for a causally valid full-repeat escape.
3. Determine tokens, language, strategy, and adaptive bucket.
4. Apply the safe effective threshold without going below configuration.
5. Run the existing deterministic compressor and 30% savings floor.

After successful compression, append the enriched `save` record. Targeted Reads continue to pass through before any compression work, then receive recovery classification for accounting only.

## 12. CLI surface

Add:

```text
ctk stats [--json]
ctk adaptive status [--json]
ctk adaptive explain <file>
ctk adaptive reset
ctk output status
```

`ctk adaptive status` shows mode, reset epoch, policy version, bucket samples, recovery overhead, and effective multiplier.

`ctk adaptive explain <file>` ships with safe mode and shows the file's language, size bucket, configured threshold, effective threshold, observations, recovery overhead, and resulting decision.

`ctk output status` shows the configured profile and estimated visible output tokens. It does not claim authoritative API usage.

Update `ctk init`, `ctk uninstall`, and `ctk doctor` to manage every cubtoken-owned event entry without touching other hooks. Keep the plugin hook manifest equal to the CLI-installed event set.

## 13. File-by-file implementation

### `ctk-compress`

- `read.rs`: add `ReadMetadata`, derive elided ranges, compute fingerprint, and accept an effective threshold without mutating global config.
- `config.rs`: add strict `AdaptiveCfg`, `AdaptiveMode`, and `OutputCfg`; default adaptive behavior to `observe` only after observation is complete, and output behavior to `default`.
- `ctk-sitter/src/lib.rs`: add structural elided ranges at the existing gap-emission point; defer profile policy.

### `ctk-hook`

- `protocol.rs`: tolerant event envelope and legacy PostToolUse inference.
- `ledger.rs`: optional record metadata, turn/batch markers, output token counts, normalized lookup, locking, and old-record parsing.
- New `attribution.rs`: pure overlap, returned-range, fingerprint, and confidence functions.
- New `adaptive.rs`: pure bucket aggregation and recommendation functions; filesystem work remains outside it.

### `ctk-cli`

- `main.rs`: `--json` stats plus adaptive and output command groups.
- `stats.rs`: legacy, attributed-context, and estimated visible-output metrics with one serializable report model.
- `init.rs` and `doctor.rs`: install, remove, and verify the complete event set.

### Plugin

- Update `plugins/cubtoken/hooks/hooks.json` to the same event set.
- Preserve the existing PostToolUse matcher for Read, Grep, Glob, Bash, Edit, Write, and NotebookEdit.
- Do not add matchers to events that do not support them.

## 14. Shipping phases

### Phase 0 — baseline

Capture current Read fixtures, compression ratios, ledger replay cost, hook latency, and end-to-end task results. No adaptive behavior ships before this baseline exists.

### Phase 1 — causal observation

Add event-aware parsing, turn and batch boundaries, decision metadata, normalized identity, fingerprints, and elided ranges. Compressed tool output must remain byte-for-byte identical.

### Phase 2 — attributed reporting

Add targeted and full-repeat classification, confidence levels, `ctk stats --json`, and backward-compatible metrics. Behavior remains static.

### Phase 3 — immediate escape and observe mode

Pass through a causally valid repeated full Read. Add deterministic snapshots, status/reset, and recommendations that do not affect first-read compression.

### Phase 4 — safe one-way backoff

Enable `safe` mode after observation data calibrates the thresholds. Dogfood it before considering a default change.

### Phase 5 — Net Task Tokens benchmark

Run matched successful tasks with cubtoken off, static, observe, and safe. Capture authoritative input, cache-read, cache-write, and output usage. Publish task-level results, not payload-compression percentages.

### Phase 6 — opt-in output discipline

Add `SessionStart`, `[output].mode = "concise"`, visible-output estimates, and an isolated benchmark arm. Keep it opt-in until task success and corrective-turn rates are non-inferior.

Graduated profiles, probation reads, adaptive output learning, and adaptive non-Read tools require separate evidence and plans.

## 15. Acceptance tests

The minimum new coverage is:

- same-batch Reads never attribute recovery; a later-batch overlapping Read can
- changed content, alias paths, and intervening edits prevent high confidence
- reset remains effective after aggregation while historical stats remain readable
- safe mode never compresses where static configuration would pass through
- concurrent ledger writes remain parseable and old `edit`/`save`/`refetch` records load

Keep the existing invariant test: every source line is either shown verbatim or covered by an advertised elided range.

The Output Governor additionally needs one integration test proving `default` emits nothing and `concise` injects its instruction once at `SessionStart`.

## 16. Performance and live validation

Benchmark ledger replay, append, lookup, attribution, and aggregate rebuild at 100, 1,000, 5,000, and 10,000 records.

Targets:

```text
p95 governor overhead, excluding parsing/compression:
  < 2 ms for a typical session
  < 5 ms for a large session
```

If those targets fail, add the smallest indexed session snapshot that fixes the measured bottleneck.

Run disposable-repository task comparisons in this order:

1. cubtoken off
2. current static cubtoken
3. adaptive observe mode
4. adaptive safe mode
5. adaptive safe mode plus opt-in concise output

Measure task success first, then authoritative total input tokens, total output tokens, Net Task Tokens, tool-result tokens, context regret, Read count, corrective turns, elapsed time, and hook overhead.

Run the output arm only after the first four context arms establish a stable baseline. Report input, cache-read, cache-write, and output deltas separately so the causal story remains clear.

## 17. Success criteria

Ship safe mode only when:

- compressed output preserves every current correctness invariant
- high-confidence attribution excludes same-batch and changed-file Reads
- reset, corruption, concurrency, and old-ledger behavior are tested
- net context saved and Net Task Token savings remain positive on representative successful tasks
- p95 hook overhead meets the stated targets

Ship opt-in output discipline only when it reduces authoritative output tokens without increasing task failures or erasing the gain through corrective turns.

The product claim is then:

> cubtoken is a native Claude Code context governor: it measures whether compression actually helped, reports Net Context Saved, and safely backs off when Claude needs context back. Optional output discipline can reduce response tokens without changing that core identity.
