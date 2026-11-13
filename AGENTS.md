# OpenVideoLibrary agent contract

- Rust is the chosen product backend; no Go source or Go builds belong in this repository. This is not a measured performance-winner claim.
- Read docs/STATUS.md before features or release claims. Keep docs/PARITY.md and docs/VALIDATION.md honest about current Rust evidence.
- Optimize measured bottlenecks and preserve benchmark semantics. Report hardware, workload, errors and uncertainty. Historical prototype reports do not certify production.
- Media mounts are read-only. Missing/empty NAS mounts must never trigger destructive reconciliation. Resolve symlinks and enforce current library permissions for every media-related endpoint, including lists and playback sessions.
- Bound scans, subprocesses, provider calls and temporary storage. Cancellation must terminate and reap subprocesses; shutdown waits for tracked tasks.
- docs/openapi.json owns public types. Run python3 scripts/generate-types.py after changes and update endpoint tests.
- Keep existing SQLite v1/v2 data, bcrypt passwords, sessions, grants and backups compatible. Test on copied/disposable databases.
- Run verification locally and over SSH on N100. Hosted GitHub Actions are disabled by user direction; do not re-enable or dispatch them without explicit approval to spend Actions minutes. Keep blocked platform checks explicit rather than substituting a hosted run.
- Verify with npm run check --prefix web, npm run build --prefix web, cargo fmt --all -- --check, cargo clippy --locked --workspace --all-targets -- -D warnings and cargo test --locked --workspace.
- Run generated-media tests explicitly with OVL_ACCELERATOR=cpu cargo test --locked --workspace -- --ignored. GPU and real-device tests are separate acceptance.
- Never label unrun browser/GPU/platform benchmarks as passing. No fabricated speed or concurrency claims.

## Required adversarial review

- Every implementing or coordinating agent run must obtain an independent adversarial review from a separate sub-agent before its final handoff. Review-only sub-agents return findings to their parent and are exempt from spawning their own reviewers or independently rerunning the full test inventory; the parent owns those completion requirements. This explicitly authorizes spawning the reviewer. Give it the task requirements, changed files/diff, relevant contracts and validation evidence; it must inspect the actual work rather than rely on the implementation summary.
- The reviewer must actively seek counterexamples and regressions in correctness, security, permissions, migration/data preservation, concurrency, cancellation, resource bounds, performance and test coverage. For documentation-only work, review the instructions, claims and consistency with the repository.
- Keep the reviewer read-only. The implementing agent owns fixes. Address every actionable finding, add regression tests where relevant, rerun affected checks and obtain a follow-up review of the fixes. Report any unresolved finding and its impact explicitly; do not claim completion while a blocking finding remains.
- Summarize the independent review and disposition of findings in the final handoff. If sub-agents are unavailable, state that the required independent review is blocked; self-review is additional evidence, not a substitute.

## Performance is a first-class requirement

- Prioritize latency, sustained throughput, CPU/RAM, startup/seek/stall behavior and bounded resource use alongside correctness and security. Do not trade away permissions, durability, media quality or equivalent workload semantics for a faster result.
- For performance-sensitive changes, establish a reproducible baseline, profile the suspected bottleneck and compare before/after under equivalent workloads. Include relevant background work, errors, resource use and repeated-run uncertainty. Investigate regressions before accepting the change.
- Keep expensive/blocking work off async request executors and preserve explicit admission limits, backpressure and cancellation. Identify missing measurements rather than assuming Rust, a library change or a microbenchmark proves an improvement.

## Tests and completion evidence

- Add or update relevant tests for new behavior, contract changes and bug fixes. Reproduce bugs with regression tests where feasible. Cover failure paths, boundaries, permissions, concurrency/cancellation and resource limits when the change affects them; prefer behavioral assertions over implementation-mirroring tests.
- Before the implementing/coordinating agent’s final handoff, discover and run all available repository test suites and required checks, not just tests for touched files. Use AGENTS.md, build targets, CI workflows, package manifests and validation documentation to inventory unit/integration tests, Python tests, frontend checks/builds, contract-generation checks, browser tests and explicitly ignored/generated-media tests. Include available platform/GPU/benchmark acceptance checks when their prerequisites and authorization are present.
- Use isolated disposable data and read-only media fixtures. Never run destructive tests against existing libraries, production databases or shared services. Request any genuinely required external authorization rather than treating this policy as permission for destructive actions.
- A missing tool, device, credential, storage allocation or authorization makes that check blocked or unrun, never passed. Run all independent checks that remain possible. Do not silently skip, weaken or disable a failing test; investigate and report any pre-existing failure separately with evidence.
- After fixes, rerun affected tests; rerun the full suite if changes invalidate the earlier full run. The final handoff must list suites run, results, added/updated coverage, adversarial review outcomes and every blocked/unrun check with its reason. Keep validation documentation consistent with the evidence.
