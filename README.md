# freenet-todo-list

A minimal Freenet dApp: a **shared todo list** where anyone with an ed25519
keypair can add and toggle tasks (multi-author, contract state), plus
**private per-task notes** that live only on the user's device via a
delegate (local secrets). Exercises all three Freenet components —
contract, delegate, UI — with the minimum viable surface area.

The contract state is a commutative monoid: `tasks` is an additive set
(`BTreeMap<id, SignedTask>`, union on merge), `toggles` is
last-writer-wins by timestamp with a deterministic signature tie-break.
Every field is signed, so untrusted peers cannot forge entries. The
delegate stores notes in its secrets namespace keyed by
`note:<contract-instance>:<task_id>`, isolated per app via
`MessageOrigin::WebApp`.

See `TUTORIAL.md` for the design walkthrough (decomposition heuristics,
the four `ContractInterface` state APIs, the commutative-monoid
requirement, pitfalls).

## Layout

```
common/                     shared Rust types (SignedTask, SignedToggle, TodoState)
contracts/todo-contract/    the shared list contract (WASM)
delegates/notes-delegate/   per-user private notes delegate (WASM)
ui/                         Rust core (wasm) + plain HTML/JS UI
  src/lib.rs                #[wasm_bindgen] facade: WebApi, signing, delegate messaging
  static/index.html         HTML + CSS
  static/app.js             plain JS: DOM rendering + event binding
```

The UI is a **Rust core compiled to wasm** (handles WebSocket, `WebApi`,
`ed25519-dalek` signing, delegate messaging via native `freenet-stdlib`)
with a **plain HTML/JS frontend** (no framework, no npm). This gives
native Freenet API access without the FlatBuffers hacks or type-mirroring
required by a pure-TS approach, and without the build-toolchain friction
of Dioxus.

## Prerequisites

- Rust + `wasm32-unknown-unknown` target (`rustup target add wasm32-unknown-unknown`)
- `wasm-bindgen-cli` (`cargo install wasm-bindgen-cli`)
- `fdev` (`cargo install fdev`)
- A running local-mode Freenet node (see "Run the UI" below)

## Setup

```sh
# build the contract and delegate WASM (run from each package dir)
cd contracts/todo-contract && fdev build --package-type contract && cd -
cd delegates/notes-delegate && fdev build --package-type delegate && cd -
```

## Test

```sh
cargo test -p todo-common   # signature roundtrip, merge commutativity, tamper rejection
```

## Publish to the local node

Start a local-mode node (network-mode doesn't register contract code for
local execution, so updates fail with "Contract not in store"):

```sh
freenet local --ws-api-port 7510
```

Publish the contract and delegate:

```sh
printf '{"tasks":{},"toggles":{}}' > /tmp/empty_todo.json

fdev -p 7510 publish --code contracts/todo-contract/build/freenet/todo_contract \
    contract --state /tmp/empty_todo.json
# → prints the contract key; copy for the next step

fdev -p 7510 publish --code delegates/notes-delegate/build/freenet/notes_delegate delegate
# → prints the delegate key; copy for the next step
```

Get the code hashes:

```sh
fdev inspect contracts/todo-contract/build/freenet/todo_contract code
fdev inspect delegates/notes-delegate/build/freenet/notes_delegate code
```

## Inject keys into the UI

The Rust core reads keys via `include_str!` at compile time. Write them
as plain base58 text files in `ui/`:

```sh
echo "<YOUR_CONTRACT_KEY>"        > ui/todo_contract_key.txt
echo "<YOUR_CONTRACT_CODE_HASH>"  > ui/todo_code_hash.txt
echo "<YOUR_DELEGATE_KEY>"        > ui/delegate_key.txt
echo "<YOUR_DELEGATE_CODE_HASH>"  > ui/delegate_code_hash.txt
```

These files are gitignored — instance-specific to your node.

## Build and publish the webapp

```sh
fdev -p 7510 website init todo    # one-time: generate signing keypair

cargo build -p todo-ui --target wasm32-unknown-unknown --release
wasm-bindgen --target web --out-dir ui/dist \
    target/wasm32-unknown-unknown/release/todo_ui.wasm
cp ui/static/index.html ui/static/app.js ui/dist/

fdev -p 7510 website publish ui/dist --key todo
# → http://127.0.0.1:7510/v1/contract/web/<KEY>/
```

### Iterate on UI changes

After editing `ui/src/lib.rs` or `ui/static/app.js`:

```sh
cargo build -p todo-ui --target wasm32-unknown-unknown --release && \
wasm-bindgen --target web --out-dir ui/dist \
    target/wasm32-unknown-unknown/release/todo_ui.wasm && \
cp ui/static/index.html ui/static/app.js ui/dist/ && \
fdev -p 7510 website update ui/dist --key todo --timeout 120
```

Then hard-refresh the browser. There is no dev server or HMR — the sandbox
iframe's WebSocket shim requires the shell page, which only exists when
served by the node.

## Architecture: Rust core + JS UI

The Rust core (`ui/src/lib.rs`) exposes a small `#[wasm_bindgen]` facade:

- `connect()` — opens the WebSocket, loads identity from delegate, subscribes
- `add_task(text)` — signs and sends a task delta
- `toggle_task(id)` — signs and sends a toggle delta
- `save_note(id, text)` / `load_note(id)` / `list_notes()` — delegate messaging
- `rotate_key()` — generates a new identity and saves it to the delegate

Callbacks (JS functions the Rust core calls):

- `todoOnState(json)` — contract state or delta update
- `todoOnStatus(status)` — connection status change
- `todoOnDelegate(json)` — delegate response (note saved, note loaded, etc.)
- `todoOnIdentity(shortId)` — identity loaded/generated

The JS side (`ui/static/app.js`) is plain DOM manipulation and event
binding. No framework, no build step, no npm dependencies.

## Publish to the network (production)

Publishing to a network-mode node makes the contract, delegate, and webapp
available to other peers on the Freenet network. The steps are the same as
the local flow, but against the network node (default port 7509, no `-p`
flag):

```sh
cd contracts/todo-contract && fdev build --package-type contract && cd -
cd delegates/notes-delegate && fdev build --package-type delegate && cd -
cargo build -p todo-ui --target wasm32-unknown-unknown --release
wasm-bindgen --target web --out-dir ui/dist \
    target/wasm32-unknown-unknown/release/todo_ui.wasm
cp ui/static/index.html ui/static/app.js ui/dist/

printf '{"tasks":{},"toggles":{}}' > /tmp/empty_todo.json
fdev publish --code contracts/todo-contract/build/freenet/todo_contract \
    contract --state /tmp/empty_todo.json
fdev publish --code delegates/notes-delegate/build/freenet/notes_delegate delegate

fdev website publish ui/dist --key todo
# → http://127.0.0.1:7509/v1/contract/web/<KEY>/
```

For a production gateway with `hosted-mode = true`, the shell mints
per-user auth tokens, giving each browser window its own delegate-secret
namespace (true per-user private notes). With `hosted-mode = false` (the
default for local dev), all connections share one `Local` namespace.

## Notes / known limitations

- **Naive full-state deltas.** `summarize_state` returns the full state
  bytes. Fine for a tiny list; compact summaries would be the next step.
- **One global shared list.** Contract parameters are empty. Distinct
  lists would be separate contract instances with distinct parameters.
- **Delegate WASM upgrades** re-key the delegate and strand stored
  secrets. A `legacy_delegates.toml` registry + backward-probe migration
  is the production answer; not wired up here.
- **Toggle always sets `done = true`.** The JS side tracks current state
  but doesn't communicate it back to Rust. A proper toggle would read the
  current state in Rust or accept the desired value as a parameter.
