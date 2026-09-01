---
name: code-audit-scan
description: >
  General-purpose methodology for auditing a codebase for security vulnerabilities
  and logic defects, validating fixes, and system-testing changes. ACTIVATE when asked
  to "scan", "audit", "review for bugs/vulnerabilities", "security review", "find logic
  problems", "整理清单", or to system-test / verify a change set. Drives a category-by-
  category, item-by-item pass grounded in file:line evidence, rated by severity AND
  confidence, and verified by build + tests + (when relevant) runtime exercise. For a
  specific project, layer that project's own audit sub-skill on top of this one.
---

# Code Audit & Scan — general methodology

A disciplined, repeatable pass for finding real vulnerabilities and logic defects,
proving fixes work, and testing behavior. Optimized for signal over volume: every
finding is grounded in `file:line`, rated by severity **and** confidence, and paired
with a concrete fix.

## Operating principles

1. **Evidence or it didn't happen.** Every finding cites `file:line` and the exact
   mechanism (the source→sink path). No "might be vulnerable" without showing the path.
2. **Severity and confidence are separate axes.** A high-impact guess is not the same
   as a proven low-impact bug. Report both (see Phase 3). Never inflate confidence to
   match an interesting impact.
3. **Trace data, not vibes.** Follow untrusted input from its **source** (IPC/RPC arg,
   HTTP/webhook body, CLI arg, env, file, queue message, a DB row written from any of
   these, **AI/agent output**) to its **sink** (shell, SQL, filesystem, git, network,
   deserializer, template/format string, dynamic eval, another LLM prompt, an HTTP
   response/DOM). A finding is that path with a missing or wrong guard.
4. **Category-by-category, item-by-item.** Walk the Phase-2 checklist in order. For each
   category, *enumerate* every relevant site (grep), then judge each. Record "checked,
   OK" deliberately — absence of a finding must be a decision, not an oversight.
5. **Baseline before you touch anything.** Build + run existing tests first, so you can
   separate pre-existing breakage from regressions you introduce.
6. **Invariants are assertions to falsify.** Mine AGENTS.md / specs / design docs for
   the system's stated security & architecture rules; actively try to break each one.
7. **Trust nothing across a boundary**, including your *own past fix* — re-audit it.

## Modes (pick one up front; they change the entry strategy)

- **Full-tree audit** — unknown/large codebase. Map structure first (Phase 1), then
  sweep all categories. Budget reads: lean on grep + codegraph to locate, read only the
  hotspots in full.
- **Diff / PR review** — a change set. Start from `git diff`; for each changed symbol,
  pull its callers/callees and ask "what new source→sink path or broken invariant does
  this introduce?" Cheaper but must still consider interactions with *unchanged* code.
- **System test / verify** — prove behavior, not just read code (see Phase 5b). Use when
  asked to "verify", "test", "confirm the fix works", or to validate a pipeline/gate.

## Phase 0 — Scope & baseline

- Fix the scope: whole tree vs commit range vs working diff (`git status` / `git diff`).
- List source files + sizes; the largest/most central files concentrate risk.
- Run build + tests; capture the result as the control (`cargo check --tests && cargo
  test`, `npm run build && npm test`, `pytest -q`, …). Zero-warning baseline is ideal.
- Read invariant docs and any project audit sub-skill. Note the language/runtime so you
  apply its specific lenses (see "Language lenses").

## Phase 1 — Map trust boundaries & attack surface

- **Sources:** every untrusted entry — IPC/RPC commands, HTTP/webhook handlers, CLI,
  env, file/queue inputs, deserialized payloads, and **all model/agent output**.
- **Sinks:** shell/process exec, SQL, filesystem paths, git, outbound HTTP (SSRF),
  deserializers, template/format strings, dynamic code, rendered HTML/DOM, IPC/HTTP
  responses leaving the backend, secondary LLM prompts.
- **Proxies / single-entry rules:** any "all X must go through Y" (a git proxy, one IPC
  layer, a sanitizer, the only merge trigger). For each, grep for **direct uses that
  bypass it** — bypass-hunting is where the real bugs hide.
- **Secrets:** where keys/tokens/passwords live and every egress path (UI, logs, error
  strings, temp files, telemetry, events).

### Tooling playbook (how to enumerate, not just where)

Locate with grep/codegraph; read hotspots in full. Generalized bypass-hunt patterns:

```
# injection sinks
grep -rn 'Command::new\|exec(\|spawn(\|subprocess\|os.system\|child_process'   # shell/cmd
grep -rn 'format!.*SELECT\|query(.*+\|f"SELECT\|"SELECT.*" *%\|execute(.*%'     # SQL by string
grep -rn 'innerHTML\|dangerouslySetInnerHTML\|v-html\|render_template\|Template(' # XSS/SSTI
grep -rn 'pickle\|yaml.load\b\|Marshal\|ObjectInputStream\|deserialize'         # unsafe deser
# path / ssrf / secrets / control flow
grep -rn '\.\.\|join(\|canonicalize\|realpath'                                  # path traversal
grep -rn 'reqwest\|http\|requests\.\|fetch(\|axios'                             # SSRF / egress
grep -rn 'api_key\|secret\|token\|password\|authorization'                      # secret flow
grep -rn 'let _ =\|except: *pass\|catch *{ *}\|\.unwrap_or(true)'               # swallowed/permissive
grep -rn 'unwrap()\|expect(\|panic!\|\!\.\|as any'                              # panics / type escapes
```

## Phase 2 — Category checklist (walk in order)

For each: enumerate sites, judge each, record OK or a finding.

1. **Injection (cmd / SQL / NoSQL / LDAP / prompt).** External input validated/escaped
   before its sink? Parameterized queries (no string-built SQL with user data)? Commands
   built from arg vectors, not `sh -c "<interpolated>"`? Prompt-injection filters on
   *all* external + agent text, including "trusted" sources that are actually externally
   authored?
2. **AuthN / session / tokens.** Credentials verified with constant-time compare?
   Empty/missing secret = **deny**, never allow? Token/session lifecycle (expiry,
   rotation, revocation, fixation)? Predictable IDs/tokens?
3. **AuthZ / access control (incl. object-level / IDOR).** Every privileged action
   gated? Can the caller act on objects they don't own by changing an ID? Multi-tenant
   isolation? Is the "only entry point" for a dangerous op truly the only one?
4. **Path handling.** Traversal (`..`, absolute, drive/UNC prefix) AND symlink escape:
   canonicalize then assert containment. Validate *before* the FS op; re-check after
   `create_dir_all` if a parent could be a symlink.
5. **Process / command execution.** Arg vectors vs shell strings. Transport restriction
   (e.g. `GIT_ALLOW_PROTOCOL`, blocked `ext::` helpers). Process-group cleanup so child
   trees aren't orphaned; `kill_on_drop`/timeouts on every spawn.
6. **Web surface (if applicable).** XSS (reflected/stored/DOM), CSRF, SSRF + open
   redirect, SSTI, CORS misconfig, clickjacking, missing security headers, mixed
   content. For desktop/webview apps: capability scoping, navigation allowlists.
7. **Concurrency & state.** Check-then-act (TOCTOU) on shared counters/DB rows — prefer
   one atomic conditional write. Locks held across await; lock ordering. Idempotency for
   queued work. Counters incremented in one path but **not decremented in all exit
   paths** (drift → false thresholds).
8. **Resource management.** Unbounded reads into memory (size-cap before
   read/base64/decompress — guard against zip/decompression bombs). Handle/temp-file/
   child-process leaks. Missing timeouts on network + subprocess.
9. **Secrets, crypto & privacy.** Keys reaching UI/logs/errors/events. Plaintext at rest
   where it shouldn't be. Weak/home-rolled crypto, missing signature verification, no
   replay protection. PII over-collection or logging; sensitive data in error strings.
10. **Error handling & graceful degradation.** "On error → allow/skip" that turns a
    safety check into a no-op when a dependency is down. Swallowed errors hiding failed
    security-relevant writes. Stack traces / internal detail leaked outward.
11. **Business-logic / workflow gates.** Implementation matches the *intended* order
    (e.g. test-before-merge, two-stage approval, gate-then-act)? Steps skippable,
    reorderable, or replayable? Do follow-up actions actually **block**, or just log?
    Numeric/limit logic (off-by-one, integer overflow, negative/zero, rounding, race on
    balance/quota)?
12. **Deserialization / parsing / config.** Untrusted YAML/JSON/XML into types with side
    effects or gadget chains. Config that becomes a shell command or a network target.
13. **Dependencies & build.** Version pins honored; no forbidden frameworks/APIs; known
    CVEs in deps; migrations additive-only (never edit an applied migration); no secrets
    in committed config; reproducible/locked builds.

### Language lenses (apply the relevant ones)

- **Rust:** `unwrap/expect/panic!` on untrusted paths; `let _ =` swallowing security
  writes; `kill_on_drop`/process groups; integer overflow in release; `unsafe` blocks;
  blocking calls in async; held `Mutex` across `.await`.
- **JS/TS:** `innerHTML`/`dangerouslySetInnerHTML`, prototype pollution, `eval`/`Function`,
  `==` auth checks, missing `await` on a guard, `as any` hiding unsafe casts.
- **Python:** `yaml.load` (not safe_load), `pickle`, f-string SQL, `subprocess(shell=True)`,
  `except: pass`, mutable default args.
- **Go:** ignored errors (`_`), goroutine leaks, missing `ctx` cancellation, `defer` in loops.
- **SQL:** identifier vs value interpolation (bind values; allowlist identifiers).

## Phase 3 — Rate each finding (severity × confidence)

**Severity** = exploitability × impact:

| Severity | Meaning |
|----------|---------|
| 🔴 Critical | Remote/unauth RCE, auth bypass, mass data loss, or a core invariant fully defeated; exploit is straightforward. |
| 🟠 High | Real exploit with a precondition; privilege/gate bypass; corruption of a protected resource. |
| 🟡 Medium | Needs local/authenticated access or fragile preconditions; defense-in-depth gap; logic bug with limited blast radius. |
| 🔵 Low / Info | Hardening, theoretical, or quality issue; correct-but-fragile code. |

**Confidence** = how sure you are the path is real and reachable:

| Confidence | Meaning |
|------------|---------|
| Confirmed | Traced end-to-end (or reproduced). |
| Likely | Path is clear but one link unverified (e.g. caller set, runtime value). |
| Speculative | Pattern-matched; needs runtime/PoC to confirm — say so explicitly. |

A fragile-but-currently-unreachable check is Medium severity at most; note *why* it's
not higher (e.g. "all callers are hardcoded"). Don't drop it — record it as Low + the
reachability caveat.

## Phase 4 — Report

Group by severity. Finding template:

```
[SEV · Confidence] file:line — short title            (invariant #N if it violates one)
  Mechanism : the source→sink path / broken guard, concretely.
  Impact    : what an attacker/operator gets; preconditions.
  PoC       : (High/Critical) a concrete attack path or input.
  Fix       : the specific change, at the right layer.
```

Then a **Coverage ledger**: list each category / invariant and mark
covered ✓ / not-applicable / not-reached, plus the "checked, no issue" areas. This makes
completeness auditable and tells the reader what you did *not* look at. Restate key
results in prose — the reader may not see tool output.

**Completion criteria:** every Phase-2 category enumerated (or marked N/A), every
project invariant tested, every trust-boundary proxy checked for bypass, baseline + post
build/test green.

## Phase 5a — Fix & verify (when asked to fix)

- Fix at the **right layer**: a missing shared guard, not one call site. If the same
  pattern recurs, fix the helper and route callers through it.
- Smallest blast radius first; keep edits idiomatic to surrounding code.
- **Re-audit the fix's interactions.** A fix is a change → re-run this skill on what it
  touches. Ask: does this new guard/counter/ordering interact with an *existing* feature
  in a way that creates a new leak/race/bypass? (Real example: wiring a counter on the
  "normal" success path leaked it on a pre-existing "auto" path that skipped the
  decrement.) Enumerate every exit path of the code you changed.
- Add a **regression test** that fails before and passes after, especially for reordered
  gates and new guards. For behavior you can't unit-test, prove the path another way
  (Phase 5b) or trace the argument showing the guard now fires.
- Re-run build + tests; **zero new warnings**. Update the invariant doc/sub-skill if the
  fix changes a rule.
- Don't say "fixed" without the build/test evidence in hand.

## Phase 5b — System test / runtime verification

When the ask includes testing behavior (not just static review):

- Identify the exact behavior to prove and its observable signal (exit code, DB state,
  emitted event, HTTP status, log line, UI state).
- Exercise the real path — run the app/CLI/test that triggers it; for env-specific
  features use the correct harness (e.g. a full desktop/IPC run, not a browser stub).
- Test the **negative** too: the guard *blocks* the bad input, not just allows the good.
- Cover boundaries: empty, max, malformed, concurrent, dependency-down.
- Report observed vs expected for each case; a fix isn't verified until the failing case
  is shown to now behave correctly.

## Anti-patterns to avoid

- Dumping every grep hit as a "finding" (volume ≠ value); not separating confidence from severity.
- Claiming a vuln without the source→sink path, or asserting "secure" without enumerating the sites.
- Skipping the baseline, then blaming your edit for pre-existing failures.
- Fixing the symptom (one call site) when the bug is a missing shared guard.
- Shipping a fix without re-auditing how it interacts with existing features and exit paths.
- Saying "done" without a build/test run (and, for behavioral changes, a runtime check).
