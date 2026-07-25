# AGENTS.md

Project-specific notes for agents working on this repo. For environment
setup (local node, binary paths, stdlib API, verified gotchas), load the
`freenet-dev` opencode skill first.

## Test

```sh
cargo test -p todo-common
```

## Key patterns

- **Commutative merge**: `TodoState::merge` is union for tasks (additive
  set), LWW by `(ts, signature)` for toggles. The tie-break on signature
  bytes makes the order total. Tested for commutativity in `common/src/lib.rs`.
- **BTreeMap everywhere**: never `HashMap` in state — core's convergence
  check is byte-level and `HashMap` iteration order is nondeterministic.
- **Sign everything**: every `SignedTask` and `SignedToggle` carries an
  ed25519 signature. `validate_state` re-verifies all signatures on receipt.
