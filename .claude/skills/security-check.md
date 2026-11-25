---
description: 'Evaluate code for security risks. Use when: security review, security audit, check for vulnerabilities, injection risk, input validation, secret handling, check security, hostile input.'
---

# Security Check

Evaluate the provided code for security risks. Assume hostile input at all boundaries — Service Bus message bodies, connection strings, API responses, and user input from the TUI are all untrusted.

## Terminal Injection

This is the highest-priority risk in a TUI application.

- **All Service Bus message content must pass through `sanitize_for_terminal()` before rendering.** Message bodies, custom properties, message IDs, correlation IDs — anything that arrived over the network.
- `sanitize_for_terminal()` strips CSI (`\x1b[`), OSC (`\x1b]`), and other ANSI escape sequences that could hijack the terminal, rewrite files, or execute commands on some terminal emulators.
- Missing sanitization = terminal injection vulnerability. Flag this as Critical.

```rust
// Required before any render of external content
use crate::ui::sanitize::sanitize_for_terminal;
let safe = sanitize_for_terminal(&msg.body, true);
```

## Input Validation

- Connection strings: must be fully parsed and validated before use. Partial parsing that falls back to defaults is a misconfiguration risk.
- Namespace names and entity paths: validate format before constructing URLs. An attacker-controlled path segment could redirect requests.
- Message count inputs (peek count, bulk operation counts): validate range before use. Unchecked values passed to loop bounds cause runaway operations.
- TOML config values loaded from disk: treat as untrusted — a tampered config file is a realistic attack vector on shared systems.

## Secret Handling

- SAS keys and Azure AD tokens must never appear in:
  - Log output (`tracing` spans or events)
  - Error messages returned to the UI
  - Status bar messages
  - Panic messages
- When logging auth errors, log the error category only — not the token or key value
- Connection strings stored in TOML config are sensitive — verify they are stored in OS-appropriate locations (`config.rs:dirs_fallback()`) and not in the working directory or repo

## HTTP Response Trust

- Always check `resp.status().as_u16()` before parsing the body
- A `200 OK` with an unexpected body should fail loudly, not silently succeed with zero data
- `404` responses must be mapped to `ServiceBusError::NotFound` — not treated as empty results
- Azure API error bodies contain diagnostic info — include them in `ServiceBusError::Api { status, body }` for debuggability, but never forward them raw to the UI without sanitization

## Deserialization

- Untrusted JSON from Service Bus data plane: `serde_json` deserializes into typed structs — verify field types match expectations before use
- ATOM XML from management plane: parsed with raw string extraction — validate that extracted values are within expected ranges before use
- `BrokerProperties` header is JSON — deserialize defensively, treat missing fields as `None` not as errors

## Concurrency and State

- Race condition between `bg_running` flag and operation dispatch: the flag must be checked when the command is dispatched (`commands::dispatch` rejects exclusive commands while `bg_running`). The current architecture (single-threaded event loop) makes this safe — do not introduce multi-threaded access to `App` state.
- Cancel tokens: verify that cancelled operations do not leave `bg_running = true` permanently (would lock the UI)

## Checklist

- [ ] All Service Bus message content passes through `sanitize_for_terminal()` before render
- [ ] No secrets (keys, tokens, connection strings) in log output or error messages
- [ ] HTTP response status checked before body is parsed
- [ ] 404 responses handled distinctly from other errors
- [ ] User-supplied counts and paths validated before use
- [ ] Config values loaded from disk treated as untrusted
- [ ] `bg_running` cannot be permanently stuck `true` after cancellation or error
