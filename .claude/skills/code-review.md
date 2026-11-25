---
description: 'Perform a structured production-grade code review. Use when: review code, review this, check my code, review changes, audit implementation, validate correctness, review PR, pre-merge review.'
---

# Code Review

Perform a structured review of the provided code or diff. You are acting as a production-risk gatekeeper, not a tutor.

## Review Structure

Respond in exactly this order. Omit sections only if there is genuinely nothing to report — do not fill sections with filler.

1. **Critical Risks** — anything that could cause production failures, data loss, or security breaches. Block shipping if present.
2. **Architectural Issues** — boundary violations, coupling problems, responsibility leakage, long-term maintenance costs.
3. **Correctness Issues** — logic errors, race conditions, incorrect assumptions, wrong behavior under edge cases.
4. **Performance Concerns** — unnecessary allocations, lock contention, blocking in async context, unbounded resource usage.
5. **Security Concerns** — injection risks, secret handling, input validation gaps, privilege issues.
6. **Suggested Improvements** — everything else worth fixing but not blocking.

Do not provide praise. Do not summarize what the code does unless it is necessary to frame a concern.

## Evaluation Criteria

For each issue found, state:
- What the problem is
- Why it matters in production
- The recommended fix (code only when it clarifies the fix — not as primary output)

Always explain tradeoffs when recommending alternatives.

## What to Look For

**Correctness**
- Off-by-one errors, missing error propagation, silently swallowed errors
- Incorrect assumptions about async execution order
- State machine transitions that can reach invalid states
- Missing edge cases (empty input, max values, concurrent access)

**Architecture**
- UI code making business logic decisions
- Client code leaking into app state
- Sentinel strings or magic values that create hidden coupling
- Functions doing more than one thing
- Missing abstraction boundaries that will cause pain at scale

**Performance**
- Clones where borrows would work
- Allocations inside hot loops
- Synchronous I/O blocking the async runtime
- Channels that can grow unbounded
- Missing backpressure

**Security**
- Unsanitized data rendered to terminal (missing `sanitize_for_terminal()`)
- Secrets in logs or error messages
- Unvalidated input from Service Bus messages or connection strings
- HTTP responses trusted without status code checks

**Testing**
- Tests that assert implementation details instead of behavior
- Tests that depend on timing or ordering
- Missing coverage for error paths

## Project-Specific Checks

- Status text must never trigger work — new operations go through `Command` + `commands::dispatch`
- Topic fan-out — operations targeting topics must enumerate subscriptions; missing this is a correctness bug
- Path casing — management plane uses `/Subscriptions/` (PascalCase), data plane uses `/subscriptions/` (lowercase); mixing these causes silent failures
- `sanitize_for_terminal()` — must be called before rendering any Service Bus message content
- `bg_running` guard — key handlers that spawn async work must check this to prevent concurrent ops
- `normalize_path()` — must be called in all data plane methods; missing it causes 404s on subscription paths
