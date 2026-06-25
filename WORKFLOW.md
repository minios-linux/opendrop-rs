# Development workflow — TDD with staged commits

Both agent crates (`awdl-rs`, `airdrop-rs`) are developed **test-first**, with a
git commit at **each TDD stage**. This keeps the parallel work reviewable and
lets the orchestrator (Claude) integrate confidently.

## Branches (avoid stepping on each other)
- gpt-5.5 works on branch **`feat/awdl-rs`**, only under `crates/awdl-rs/`.
- GLM-5.2 works on branch **`feat/airdrop-rs`**, only under `crates/airdrop-rs/`.
- Claude owns `main`, the workspace `Cargo.toml`, docs, and merges.
- Do not edit files outside your crate without asking Claude (they're shared).

## The TDD cycle — commit at every stage
For each small unit of behavior:

1. **RED** — write a failing test that specifies the behavior. Run it, confirm it
   fails for the right reason. Commit:
   `git commit -m "red(<crate>): <behavior>"`
2. **GREEN** — write the minimal code to make the test pass. Run the test, confirm
   green (and that you didn't break others). Commit:
   `git commit -m "green(<crate>): <behavior>"`
3. **REVIEW / REFACTOR** — if review feedback or a cleanup is warranted (dedupe,
   naming, error handling, docs), do it with tests still green. Commit:
   `git commit -m "refactor(<crate>): <what>"` or `review(<crate>): <what>`.

Keep commits small and stage-pure: a RED commit contains only the failing
test(s); the GREEN commit contains the implementation that makes them pass. This
makes the diff at each stage tell a clean story.

## Testing conventions
- Unit tests inline (`#[cfg(test)] mod tests`) for parsers/codecs.
- Use **captured real-world fixtures** for the tricky formats — we have ground
  truth:
  - dvzip / CPIO: `/tmp/upload.bin` (a real iOS-18 dvzip), `opendrop-patch/dvzip.py`
    (reference decoder). Save fixtures under `crates/<crate>/tests/fixtures/`.
  - AWDL frames: capture with `tcpdump -i <mon> -w` on channel 44, or pull sample
    PSF/MIF bytes from owl's logs/test vectors.
- Prefer pure, testable functions for protocol parsing/encoding; isolate I/O
  (sockets, nl80211, pcap) behind small traits so logic is unit-testable without
  hardware.
- `cargo test -p <crate>` must pass before every GREEN/refactor commit.

## Definition of done for a unit
RED + GREEN committed, `cargo test -p <crate>` green, `cargo clippy` clean for the
touched code, and a `tracing` event exists for any new observable behavior
(peers, election, sync, discovery, http phase) — observability is a feature here.
