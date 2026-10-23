# Learned log

> **File purpose**: an accumulating log of technical lessons, in a stable
> format ready for cross-repo compilation — successive repos built with this
> learning objective carry their own `learned.md`, and the compilation walks
> them reading the uniform headers. Every entry: **what happened**, **the
> rule** that avoids repeating it, and the **evidence** (run/commit/number).
>
> Entry convention: `## [topic] — title` + `**Date**: ...` + body with
> `Rule:` and `Evidence:`. Stable topics (compilation keys): `scaleway`,
> `supabase`, `ci`, `rust`, `operation`.

---

## [scaleway] scaleway_function does not deploy without `deploy = true`

**Date**: 2026-10-22/23 (v0.1.2–v0.1.5)

The `deploy` attribute of `scaleway_function` (the `scaleway` provider)
defaults to `false`: the `apply` uploads the zip and updates the
configuration, but the **build and the swap only happen with the manual
Restart of the dashboard**. With `deploy = true`, Terraform also launches the
deployment and waits for the build.

- **Rule**: in every Scaleway Functions + Terraform project, declare
  `deploy = true` from day one. The apply will take the build — that is the
  correct cost, not a hang.
- **Evidence**: deploys v0.1.2..v0.1.4 with the ghost manual step; run 13
  (v0.1.5) measured the Apply at **7 min** with `deploy = true`.

## [scaleway] The cold Rust build takes >10 min — polling the build state in the CI fails and bills

**Date**: 2026-10-22/23

The smoke's build-state polling (40×15s) ran out on v0.1.2 and v0.1.3 with
correct deploys — the signature of a run of exactly ~12 min; raising to 80
attempts only made it slower (v0.1.4 paid 20 min). With `deploy = true` the
polling became redundant: the apply already waits.

- **Rule**: **never poll the build state in the CI**. The provider's wait
  (`deploy = true`) runs inside the apply that is paid anyway; the wait in
  the CI **bills twice**: billable runner-minutes AND billable function
  invocations (every API poll and every `/health` hit), amplified every time
  the job re-runs. The post-deploy check remains a single HTTP gate with
  margins of SECONDS.
- **Evidence**: run 10 (12 min), run 11 (12 min), run 12 (20.5 min); run 14
  (v0.1.6): Apply 11.9 min + **Smoke of 12 seconds** — the run fully green.
- **Measured cost of the redundant smoke (v0.1.5)**: run 12 spent ~20.5 min
  of runner + ~86 production invocations polling a correct build.

## [scaleway] rust196 runtimes: logs and metrics vs AWS Lambda

**Date**: 2026-10-22/23 — the operator's comparison

Concrete complaints against Scaleway Functions, with AWS Lambda as the
reference:

1. **The event log carries none of its cost**: the wrapper's line
   (`Function triggered: /path`) includes neither how long the event took nor
   how many resources it consumed. In Lambda, every invocation logs a `REPORT`
   with request-id, **duration**, **billed duration**, **memory used** — all
   in the same stream.
2. **Metrics live in a separate interface**: the metrics console (Cockpit) is
   a different surface from the logs — extra steps versus Lambda (where
   CloudWatch mixes the request's logs and metrics in one place).
3. **The metrics do not measure the event's execution time either**: they show
   **CPU and memory only** — correlating latency requires inferring it (or
   timing it externally).
4. **First real measurement of the function** (the current base config,
   min_scale 0 / max_scale 1 / 128 MiB): a typical event consumed **~48 MiB of
   memory and ~20% of CPU** — the `/health` answer (0.5-0.7 s of latency
   measured from outside, largely the container's cold start).

- **Rule**: the app must self-instrument what the platform does not give: the
  `TraceLayer` (latency per request), the D18 events with their own `ts` and
  the startup DB log (PR-1) stand in for Lambda's `REPORT`. Budget the crate's
  memory with the measurement (48 MiB / 128 MiB leaves a ~3× headroom — watch
  it as the routes grow).
- **Evidence**: the Functions Logs capture with only wrapper lines; the
  48 MiB / 20% CPU measurement on the operator's panel (2026-10-22).

## [scaleway] `try_init` installs are never swallowed (and the app logs worked — it was the shipper's latency)

**Date**: 2026-10-22/23

"No app logs" turned out to be the **log shipper's sync latency of the
platform**: lines with their own timestamp appeared minutes late in the view.
The tracing subscriber works fine (JSON on stdout in the legacy runtime and
stderr in the v0.1.4 fix — both channels captured; v0.1.6's revert returned it
to stdout). The wrapper does NOT block the app's own subscriber.

- **Rule**: facing a "no logs", **the log's timestamp is the truth, not the
  time of view** — wait for the ingestion (minutes) and filter by
  instance/deployment before blaming anything. And the cross-cutting Rust
  rule: the subscriber's `try_init` failure is NEVER swallowed by `let _ =` —
  eprintln with the reason (it discriminates in one deploy: a subscriber
  conflict, or the surviving channel).
- **Evidence**: the JSON `tower_http::trace::on_failure` ERROR line with
  timestamp `16:24:31Z` coming from the OLD build; the startup DB log
  (`latency_ms: 161`) visible after the sync (2026-10-23).

## [scaleway] The apply-secrets→new-build 5xx window

**Date**: 2026-10-22

The apply swaps secrets/env BEFORE the new build is ready: the old build keeps
serving with the configuration that already changed → 500 on every cold start
until the swap (v0.1.3: the old build lost `SUPABASE_JWT_SECRET` and answered
500 across the whole window).

- **Rule**: during a deploy, a configuration-like 5xx in production may come
  FROM THE OLD BUILD — correlate the deployment id before blaming the new
  code (the window ends when the swap lands; with `deploy = true` the apply
  waits for it).
- **Evidence**: v0.1.3's 500 window (~16:39–16:55, deployment-00008).

## [supabase] JWKS/ES256: the asymmetric-key era (post-october-2025)

**Date**: 2026-10-22 (T006 amend, v0.1.3)

New Supabase projects sign with **asymmetric keys (ES256)**: the legacy
`SUPABASE_JWT_SECRET` no longer exists — the project's credential is the
**JWKS URL** (public). The verifier (`JwksVerifier`) resolves the key by
`kid`, caches the set, and re-fetches on an unknown kid (rotation without
re-deploy); a fetch failure is 503 (dependency), never 401.

- **Rule**: Supabase credential signing is never assumed HS256; the project's
  `GET /auth/v1/.well-known/jwks.json` is the project's first probe. And
  locally: the legacy goTrue **does not emit `issuer`** and its JWKS answers
  with `keys: []` (symmetric keys never publish) — the locally-tested
  protected flows use the HS256 verifier of `AppState::for_tests()`; the CI
  the cloud's real JWKS.
- **Evidence**: the JWKS of production: 1 key of EC P-256/ES256 validated via
  curl (v0.1.3, 2026-10-23).

## [supabase] Session-pooler vs transaction-pooler under serverless

**Date**: 2026-10-22/23

The IPv6-less network cannot connect directly to Supabase: the pooler is the
only route — `aws-1-eu-central-1.pooler.supabase.com`. Port **5432 = session
mode** (every connection of the pool holds a pinned session — it competes
against the project's connection cap); port **6543 = transaction mode**
(releases per transaction — the correct one for a function with an internal
pool that opens and closes).

- **Rule**: migrations (the CLI, one connection at a time) → 5432; **the API →
  6543** (the production secret, a one-line change — pending to execute at the
  closing of the US1 phase). The functions' scale NEVER competes against the
  connection cap: with instances opening/closing, session mode bites the cap.
- **Evidence**: the operator's `.env` (5432 in effect) + their own comment
  pointing 6543 for the API; TCP probe OK.

## [ci] Coverage is measured BEFORE the push and the async path is covered in the first PR

**Date**: 2026-10-22 (ratchet 92.79/93.66 and T012 at 0/105)

Two coverage failures, two rules:

1. **T012 initial**: the new handler landed **0/105 covered** because its
   runner depended on a later task (T011) — the CI ratchet fails. The fix:
   unit tests of the handler against an in-process fake (axum — a prod
   dependency; no mock-http dev-deps) and the tests that fit **now**.
2. The JWKS PR's ratchet (92.79% vs base 93.66%): the tests covered only the
   easy core; the production wiring uses another implementation than the test
   state's — the uncovered part is exactly the border between them.

- **Operator's rule**: the pre-push checklist is fmt → clippy → tests →
  **`cargo llvm-cov` with the CI's exclusions** — the push is not the moment
  to discover coverage. And if a portion depends on a later task: declare it
  in the PR, do not let the CI discover it.
- **Evidence**: handlers.rs 0%→98.4% after the fake-backed tests (the delta
  recovered the ratchet with margin: +206 covered lines).

## [ci] The integration tests ran dead: the dotenv merge order

**Date**: 2026-10-23 (harness T010)

`load_env_files` loaded `.env` first with set-if-absent: the `.env`'s **cloud**
`DATABASE_URL` won and the locality guard failed — **every integration test
skipped silently**, locally and in the CI (which does not export
`SERVICE_ROLE_KEY` anyway). The whole integration tier (T010/T011+) "passed"
without running.

- **Rule**: the dotenv merge resolves **by last write** (the test stack wins)
  and the process environment keeps the last word; the aliases
  (`SERVICE_ROLE_KEY` ← `SUPABASE_SERVICE_ROLE_KEY`) live in the harness, not
  in the app's config. A silently skipped test is NOT a green test — the
  "2 passed in 0.00s" was the trap.
- **Evidence**: the guard really running against the local `pap` stack: 2/2
  integration ok in 0.05s (2026-10-23).

## [rust/axum] Deny-by-default lives in the EXTRACTOR, not in the router

**Date**: 2026-10-23

In axum, a handler without the authentication extractor is **anonymous even
when its route hangs in a nested "protected" router**: `/v1/cv/versions/{id}`
extracted only `Path` and an anonymous request passed with 503 where FR-011
demands 401. The wiring test caught it.

- **Rule**: every protected handler carries `AuthenticatedUser` as the FIRST
  extractor — no router convention protects anything. The wiring proof (every
  protected route anonymous → 401) remains permanent.
- **Evidence**: `wiring::the_protected_routes_require_a_credential` failing
  for the version route; fixed with the extractor in `stub_version_503`.

## [operation] Stabilization tasks go FIRST, not in the polish tail

**Date**: 2026-10-23 — the operator's rule

The operation/observability tasks (the startup DB proof, the automated
deploy, the polling-free smoke, the verified logging) **belong to the FIRST
tasks of the plan** — their value and priority pay up front — so the
development of the complex tasks stays precise.

- **Rule**: when ordering the tasks of any feature/project: stability and
  observability land early. And platform findings appearing mid-cycle (like
  Scaleway's) get absorbed as early tasks of the **current cycle** — never
  into a final "polish" tail.
- **Evidence**: this session — the "small tasks" (deploy=true, smoke slim,
  the startup log, the shipper's diagnosis) delivered more operational value
  than any handler; the US1/T012 cycle started on a stabilized platform.

## [operation] PRs stack and coverage travels with its feature

**Date**: 2026-10-22/23

The flow this project already validates: a branch per task, ≤6 files,
squash/merge with manual review between PRs; the fix stacked is rebased over
the branch that reported it (the rustsec fix rebased over the PR-1 branch
that reported the advisory); the new handler's coverage travels in the SAME
PR that implements it.

- **Rule**: when a finding surfaces on ONE branch, its fix rebases over that
  branch (the stack stays linear); never from main — the review then reads as
  an incremental diff on top of what is already known.
- **Evidence**: the rebase of `fix/rustsec-2026-0385` over
  `feature/T012-startup-db-ping` after the operator's feedback.