---
name: autoforge-audit-scan
description: >
  AutoForge-specific security & logic audit sub-skill. ACTIVATE for any scan/audit/
  security-review/system-test of the AutoForge codebase (the Tauri "autonomous software
  factory"). Use TOGETHER WITH the general `code-audit-scan` skill: apply that
  methodology, then walk the AutoForge-specific trust boundaries, invariants, and
  hotspot files below, item-by-item. Convert each project invariant into an assertion
  to falsify.
---

# AutoForge audit — project-specific surface

> Prerequisite: follow **`code-audit-scan`** (phases 0–5). This sub-skill supplies
> AutoForge's trust boundaries, the invariants to falsify, the hotspot files, and the
> verification commands. Tauri is **2.x only** — never reason from 1.x APIs.

## Architecture in one breath

Tauri 2 desktop app. React/TS front end ↔ Rust back end **only via Tauri IPC**
(`src/services/index.ts` → `#[tauri::command]`). SQLite (sqlx, additive migrations).
A Tokio mpsc job runner drives the pipeline: intake → analysis → **review_1 (human)**
→ execution (Codex in a git worktree) → **review_2 (human)** → **pre-merge test
gate** → merge to `dev` → post-merge security audit. AI agents run via local `Codex`
CLI or external LLM APIs.

## Trust boundaries (sources → sinks)

- **Sources:** every `#[tauri::command]` arg; the webhook HTTP handler
  (`intake/webhook.rs`); GitHub sync (`intake/github.rs`); repo file scans
  (`intake/scanner.rs`); bulk import; chat messages + attachments; **all LLM/agent
  output** (treat as untrusted before it hits a file/DB/prompt sink); `config_yaml`
  (project-controlled, but it becomes shell commands).
- **Sinks:** `GitProxy` (git), `sh -lc` test/dev commands, worktree filesystem,
  `.autoforge/` workspace writes, SQLite, the `Codex` CLI prompt, external LLM HTTP
  endpoints, data URLs returned to the webview.
- **Proxies / single-entry rules to police:** `GitProxy` (all git), `services/index.ts`
  (all IPC), `core/security::has_obvious_injection` (all external intake),
  `review_2` approved (the only merge trigger), workspace writes confined to
  `.autoforge/docs|specs`.

## Invariants to falsify (each is a test, not a given)

1. **GitProxy is the only git path.** Grep `Command::new("git")` — every hit outside
   `core/git.rs` is a bypass to justify. No push to main/master, no force/destructive
   push, no dangerous `-c` overrides, no leading-flag smuggling of the subcommand.
2. **Merge requires human review_2 AND a passing pre-merge test gate.** The only path
   to `git merge` is `review_2` approved → merge job → `testing::run_and_gate` on the
   **worktree** returns true. A failing/absent gate must block merge. Look for any
   other path that sets `merged` or calls merge. Check `core::gate` (auto-merge/trust)
   cannot skip a human node.
3. **Workspace writes stay in `.autoforge/docs|specs`.** No `..`, no absolute/root, no
   symlink escape (canonicalize + containment). Applies to `write_workspace_file`,
   `execute_agent_writes`, and `<write-file>` parsing.
4. **All external intake is sanitized.** `has_obvious_injection` runs before insert in
   the gateway; the deeper LLM `safety_check` runs for non-trusted sources. `TRUSTED_SOURCES`
   must contain only machine-internal sources — **never** externally-authored ones
   (GitHub, webhook, user input).
5. **The code agent cannot touch remotes.** Worktree `Codex` runs with
   `--disallowedTools "Bash(git *)"` AND `GIT_ALLOW_PROTOCOL=""` so no shell-indirection
   push/fetch is possible.
6. **Secrets never reach the webview.** LLM `api_key` is masked on every command that
   returns `LlmConfig`; keys/tokens never appear in logs, error strings, or events.
7. **Concurrency admission is atomic.** Execution slots are claimed with a single
   conditional UPDATE (counts re-checked under the write lock); in-memory counters
   (`active`, `pending_review`) are incremented/decremented on **every** matching path.
8. **Migrations are additive.** Never edit an applied file in `src-tauri/migrations/`.
9. **Tauri capabilities declared.** Each JS→Rust command has a permission in
   `capabilities/main.json`; missing = runtime error, not a code bug.
10. **Subprocess hygiene.** dev-server / test / agent children run in their own process
    group and are reaped (group kill / `kill_on_drop`); no orphans, timeouts present.
11. **Path safety in project_context / materials / artifacts.** Reads go through
    canonicalize + containment; filename/identifier sanitizers neutralize `..` and
    separators; inline data URLs are size-capped.
12. **Pipeline counters balance across EVERY exit path.** The pipeline has multiple
    terminal paths per CR — `pending_review_2` (human), gate-downgrade `auto` →
    `pending_merge`, `execution_failed`, `merge_failed`, revision → back to execution.
    Any in-memory counter (`active`, `pending_review`) incremented on one path must be
    decremented on *all* of them. The auto-merge path skips review_2, so counting a
    review slot there leaks forever and eventually false-trips the pause threshold —
    a real regression class. When you touch the runner/execution/review state machine,
    enumerate every exit and prove the counters net to zero.

> When fixing anything in the execution → review → merge state machine, re-audit the
> auto-merge (gate downgrade) path specifically — it is the one that skips human review
> and is easy to forget. (See general skill Phase 5a: re-audit fix interactions.)

## Hotspot files (read these every scan)

- Security core: `core/security.rs`, `core/git.rs`, `core/concurrency.rs`,
  `core/gate.rs`, `core/mask.rs`, `core/notify.rs`, `core/event.rs`
- Pipeline: `tasks/runner.rs`, `tasks/execution.rs`, `tasks/merge.rs`,
  `tasks/testing.rs`, `tasks/analysis.rs`, `tasks/security_audit.rs`
- Agents: `agents/local_claude.rs`, `agents/code_agent.rs`, `agents/llm.rs`,
  `agents/analysis.rs`, `agents/grader.rs`
- Commands (IPC): `commands/workspace.rs`, `commands/project_context.rs`,
  `commands/projects.rs`, `commands/change_requests.rs`, `commands/orchestration.rs`,
  `commands/materials.rs`, `commands/conversations.rs`, `commands/settings.rs`,
  `commands/dev_server.rs`, `commands/specs.rs`, `commands/system.rs`
- Intake: `intake/webhook.rs`, `intake/github.rs`, `intake/scanner.rs`,
  `intake/gateway.rs`, `intake/bulk.rs`
- Knowledge layer (newer; audit prompt-injection & data flow): `knowledge*`,
  `commands/*` that call `kb_*`

## High-value greps (bypass hunting)

```
grep -rn 'Command::new'                 # git/shell bypasses, missing process groups
grep -rn 'format!.*SELECT\|query(&format' # SQL built by interpolation
grep -rn 'sh").*-lc\|-c"'               # shell-string execution from config/input
grep -rn 'let _ = '                     # swallowed errors on security-relevant writes
grep -rn 'canonicalize\|\.\.\|join('    # path handling
grep -rn 'api_key\|token\|password\|secret' # secret flow to UI/logs/errors
grep -rn 'unwrap()\|expect('            # panics on untrusted input paths
grep -rn 'TRUSTED_SOURCES\|has_obvious_injection\|safety_check' # sanitizer coverage
```

## Verify (build + tests)

```
cd src-tauri && cargo check --tests --message-format=short   # zero errors, zero warnings
cd src-tauri && cargo test --lib                              # all green
# Tauri-dependent behavior (IPC/window/FS) must be exercised via `npm run tauri:dev`,
# not the browser `npm run dev` (per testing.md).
```

## Reporting

Same format as the general skill: severity-grouped checklist, each item
`[sev] file:line — title · mechanism · impact · fix`, plus an explicit
"checked, no issue" list so coverage of the invariants above is auditable.
Cross-reference any finding to the invariant number it violates.
