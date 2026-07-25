use ed25519_dalek::{Signer, SigningKey};
use freenet_stdlib::client_api::{
    ClientRequest, ContractRequest, DelegateRequest, HostResponse, WebApi,
};
use todo_common::{SignedTask, SignedToggle, TodoState};
use std::collections::BTreeMap;
use std::cell::RefCell;
use std::sync::OnceLock;
use wasm_bindgen::prelude::*;

const TODO_CONTRACT_KEY: &str = include_str!("../todo_contract_key.txt");
const TODO_CODE_HASH: &str = include_str!("../todo_code_hash.txt");

// ---- JS callbacks ----
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = "todoOnState")]
    fn js_on_state(json: &str);

    #[wasm_bindgen(js_name = "todoOnStatus")]
    fn js_on_status(status: &str);

    #[wasm_bindgen(js_name = "todoOnDelegate")]
    fn js_on_delegate(json: &str);

    #[wasm_bindgen(js_name = "todoOnIdentity")]
    fn js_on_identity(short_id: &str);

    // Use JS Date.now() instead of freenet_stdlib::time::now() (which uses a
    // host function that only exists in the Freenet runtime, not the browser).
    #[wasm_bindgen(js_name = "Date.now")]
    fn date_now() -> f64;
}

// ---- Global state (thread_local for non-Send WebApi) ----
thread_local! {
    static API: RefCell<Option<WebApi>> = const { RefCell::new(None) };
    static IDENTITY: RefCell<Option<SigningKey>> = const { RefCell::new(None) };
}

// ---- Panic hook ----
#[wasm_bindgen(start)]
fn run() {
    std::panic::set_hook(Box::new(|info| {
        web_sys::console::error_1(&format!("[todo] panic: {info}").into());
    }));
}

// ---- Key construction ----
fn contract_key() -> freenet_stdlib::prelude::ContractKey {
    use freenet_stdlib::prelude::{CodeHash, ContractInstanceId, ContractKey};
    let mut id = [0u8; 32];
    let mut code = [0u8; 32];
    bs58::decode(TODO_CONTRACT_KEY.trim()).onto(&mut id).unwrap();
    bs58::decode(TODO_CODE_HASH.trim()).onto(&mut code).unwrap();
    ContractKey::from_id_and_code(ContractInstanceId::new(id), CodeHash::new(code))
}

fn contract_instance_id() -> freenet_stdlib::prelude::ContractInstanceId {
    let mut id = [0u8; 32];
    bs58::decode(TODO_CONTRACT_KEY.trim()).onto(&mut id).unwrap();
    freenet_stdlib::prelude::ContractInstanceId::new(id)
}

fn delegate_key() -> freenet_stdlib::prelude::DelegateKey {
    use freenet_stdlib::prelude::{CodeHash, DelegateKey};
    let mut key = [0u8; 32];
    let mut code = [0u8; 32];
    bs58::decode(include_str!("../delegate_key.txt").trim()).onto(&mut key).unwrap();
    bs58::decode(include_str!("../delegate_code_hash.txt").trim()).onto(&mut code).unwrap();
    DelegateKey::new(key, CodeHash::new(code))
}

// ---- WebSocket URL ----
fn ws_url() -> String {
    let win = web_sys::window().expect("no window");
    let loc = win.location();
    let proto = if loc.protocol().unwrap_or_default() == "https:" { "wss:" } else { "ws:" };
    let host = loc.host().unwrap_or_default();
    format!("{proto}//{host}/v1/contract/command?encodingProtocol=native")
}

// ---- Delegate request/response types (JSON, shared with JS) ----
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
enum DReq {
    SaveNote { task_id: u64, text: String },
    GetNote { task_id: u64 },
    ListNotes,
    SaveIdentity { private_key_hex: String, public_key_hex: String },
    GetIdentity,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
enum DResp {
    Saved { task_id: u64 },
    Note { task_id: u64, text: Option<String> },
    NotesList { task_ids: Vec<u64> },
    Identity { private_key_hex: Option<String>, public_key_hex: Option<String> },
    IdentitySaved,
    Error { message: String },
}

// ---- WASM-bindgen public API ----

#[wasm_bindgen]
pub fn connect() {
    js_on_status("connecting");
    let url = ws_url();
    let ws = web_sys::WebSocket::new(&url).expect("ws");
    ws.set_binary_type(web_sys::BinaryType::Arraybuffer);

    let api = WebApi::start(
        ws,
        move |result| handle_host_response(result),
        |_| {},
        || {
            js_on_status("connected");
            // WS is OPEN now — safe to send the initial sequence.
            // (WebApi::send rejects with an error if ready_state != OPEN.)
            spawn(async {
                let mut api_opt = API.with(|a| a.borrow_mut().take());
                if let Some(api) = api_opt.as_mut() {
                    send_delegate_msg(api, &DReq::GetIdentity).await;
                    let _ = api.send(ClientRequest::ContractOp(ContractRequest::Subscribe {
                        key: contract_instance_id(),
                        summary: None,
                    })).await;
                    let _ = api.send(ClientRequest::ContractOp(ContractRequest::Get {
                        key: contract_instance_id(),
                        return_contract_code: true,
                        subscribe: true,
                        blocking_subscribe: false,
                    })).await;
                }
                API.with(|a| *a.borrow_mut() = api_opt);
            });
        },
    );
    API.with(|a| *a.borrow_mut() = Some(api));
}

#[wasm_bindgen]
pub fn add_task(text: String) {
    spawn(async move {
        let sk = match IDENTITY.with(|i| i.borrow().clone()) {
            Some(sk) => sk,
            None => return,
        };
        let now = now_millis();
        let id = now as u64 * 1000;
        let unsig = SignedTask {
            id,
            text: text.clone(),
            created_at: now,
            author: sk.verifying_key(),
            signature: ed25519_dalek::Signature::from_bytes(&[0u8; 64]),
        };
        let signature = sk.sign(&unsig.signing_bytes());
        let task = SignedTask { signature, ..unsig };

        // Optimistic: tell JS immediately
        let delta = TodoState {
            tasks: [(id, task.clone())].into_iter().collect(),
            toggles: BTreeMap::new(),
        };
        js_on_state(&serde_json::to_string(&delta).unwrap());

        // Send to network
        let mut api_opt = API.with(|a| a.borrow_mut().take());
        if let Some(api) = api_opt.as_mut() {
            let bytes = serde_json::to_vec(&delta).unwrap();
            let _ = api.send(ClientRequest::ContractOp(ContractRequest::Update {
                key: contract_key(),
                data: freenet_stdlib::prelude::UpdateData::Delta(
                    freenet_stdlib::prelude::StateDelta::from(bytes),
                ),
            })).await;
        }
        API.with(|a| *a.borrow_mut() = api_opt);
    });
}

#[wasm_bindgen]
pub fn toggle_task(task_id: u64) {
    spawn(async move {
        let sk = match IDENTITY.with(|i| i.borrow().clone()) {
            Some(sk) => sk,
            None => return,
        };
        let now = now_millis();
        let done = true; // JS side knows current state; we just toggle to done
        let unsig = SignedToggle {
            task_id,
            done,
            ts: now,
            author: sk.verifying_key(),
            signature: ed25519_dalek::Signature::from_bytes(&[0u8; 64]),
        };
        let signature = sk.sign(&unsig.signing_bytes());
        let toggle = SignedToggle { signature, ..unsig };

        let delta = TodoState {
            tasks: BTreeMap::new(),
            toggles: [(task_id, toggle)].into_iter().collect(),
        };
        js_on_state(&serde_json::to_string(&delta).unwrap());

        let mut api_opt = API.with(|a| a.borrow_mut().take());
        if let Some(api) = api_opt.as_mut() {
            let bytes = serde_json::to_vec(&delta).unwrap();
            let _ = api.send(ClientRequest::ContractOp(ContractRequest::Update {
                key: contract_key(),
                data: freenet_stdlib::prelude::UpdateData::Delta(
                    freenet_stdlib::prelude::StateDelta::from(bytes),
                ),
            })).await;
        }
        API.with(|a| *a.borrow_mut() = api_opt);
    });
}

#[wasm_bindgen]
pub fn save_note(task_id: u64, text: String) {
    spawn(async move {
        let mut api_opt = API.with(|a| a.borrow_mut().take());
        if let Some(api) = api_opt.as_mut() {
            send_delegate_msg(api, &DReq::SaveNote { task_id, text }).await;
        }
        API.with(|a| *a.borrow_mut() = api_opt);
    });
}

#[wasm_bindgen]
pub fn load_note(task_id: u64) {
    spawn(async move {
        let mut api_opt = API.with(|a| a.borrow_mut().take());
        if let Some(api) = api_opt.as_mut() {
            send_delegate_msg(api, &DReq::GetNote { task_id }).await;
        }
        API.with(|a| *a.borrow_mut() = api_opt);
    });
}

#[wasm_bindgen]
pub fn list_notes() {
    spawn(async {
        let mut api_opt = API.with(|a| a.borrow_mut().take());
        if let Some(api) = api_opt.as_mut() {
            send_delegate_msg(api, &DReq::ListNotes).await;
        }
        API.with(|a| *a.borrow_mut() = api_opt);
    });
}

#[wasm_bindgen]
pub fn rotate_key() {
    let mut rng = [0u8; 32];
    let _ = getrandom::getrandom(&mut rng);
    let sk = SigningKey::from_bytes(&rng);
    let short = hex_short(&sk.verifying_key().to_bytes());
    IDENTITY.with(|i| *i.borrow_mut() = Some(sk.clone()));
    js_on_identity(&short);
    let priv_hex = bytes_to_hex(&sk.to_bytes());
    let pub_hex = bytes_to_hex(&sk.verifying_key().to_bytes());
    spawn(async move {
        let mut api_opt = API.with(|a| a.borrow_mut().take());
        if let Some(api) = api_opt.as_mut() {
            send_delegate_msg(api, &DReq::SaveIdentity {
                private_key_hex: priv_hex,
                public_key_hex: pub_hex,
            }).await;
        }
        API.with(|a| *a.borrow_mut() = api_opt);
    });
}

// ---- Internal: host response handler ----
fn handle_host_response(result: Result<HostResponse, freenet_stdlib::client_api::ClientError>) {
    match result {
        Ok(HostResponse::ContractResponse(cr)) => {
            use freenet_stdlib::client_api::ContractResponse;
            match cr {
                ContractResponse::GetResponse { state, .. } => {
                    if let Ok(todo) = serde_json::from_slice::<TodoState>(&state) {
                        js_on_state(&serde_json::to_string(&todo).unwrap());
                        // Load notes after getting state
                        spawn(async {
                            let mut api_opt = API.with(|a| a.borrow_mut().take());
                            if let Some(api) = api_opt.as_mut() {
                                send_delegate_msg(api, &DReq::ListNotes).await;
                            }
                            API.with(|a| *a.borrow_mut() = api_opt);
                        });
                    }
                }
                ContractResponse::UpdateNotification { update, .. } => {
                    use freenet_stdlib::prelude::UpdateData;
                    match update {
                        UpdateData::Delta(d) => {
                            if let Ok(delta) = serde_json::from_slice::<TodoState>(d.as_ref()) {
                                js_on_state(&serde_json::to_string(&delta).unwrap());
                            }
                        }
                        UpdateData::State(s) => {
                            if let Ok(full) = serde_json::from_slice::<TodoState>(s.as_ref()) {
                                js_on_state(&serde_json::to_string(&full).unwrap());
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        Ok(HostResponse::DelegateResponse { values, .. }) => {
            for msg in values {
                use freenet_stdlib::prelude::OutboundDelegateMsg;
                if let OutboundDelegateMsg::ApplicationMessage(am) = msg {
                    if let Ok(resp) = serde_json::from_slice::<DResp>(am.payload.as_slice()) {
                        handle_delegate_response(resp);
                    }
                }
            }
        }
        Err(e) => web_sys::console::error_1(&format!("[todo] host error: {e}").into()),
        _ => {}
    }
}

fn handle_delegate_response(resp: DResp) {
    match resp {
        DResp::Identity { private_key_hex: Some(pk), .. } => {
            if let Ok(priv_bytes) = hex_to_bytes(&pk) {
                if priv_bytes.len() == 32 {
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&priv_bytes);
                    let sk = SigningKey::from_bytes(&arr);
                    let short = hex_short(&sk.verifying_key().to_bytes());
                    IDENTITY.with(|i| *i.borrow_mut() = Some(sk));
                    js_on_identity(&short);
                }
            }
        }
        DResp::Identity { private_key_hex: None, .. } => {
            // No stored identity — generate and save
            let mut rng = [0u8; 32];
            let _ = getrandom::getrandom(&mut rng);
            let sk = SigningKey::from_bytes(&rng);
            let short = hex_short(&sk.verifying_key().to_bytes());
            let priv_hex = bytes_to_hex(&sk.to_bytes());
            let pub_hex = bytes_to_hex(&sk.verifying_key().to_bytes());
            IDENTITY.with(|i| *i.borrow_mut() = Some(sk));
            js_on_identity(&short);
            spawn(async move {
                let mut api_opt = API.with(|a| a.borrow_mut().take());
                if let Some(api) = api_opt.as_mut() {
                    send_delegate_msg(api, &DReq::SaveIdentity {
                        private_key_hex: priv_hex,
                        public_key_hex: pub_hex,
                    }).await;
                }
                API.with(|a| *a.borrow_mut() = api_opt);
            });
        }
        DResp::Note { task_id, text } => {
            let json = serde_json::to_string(&DResp::Note { task_id, text }).unwrap();
            js_on_delegate(&json);
        }
        DResp::NotesList { task_ids } => {
            let json = serde_json::to_string(&DResp::NotesList { task_ids: task_ids.clone() }).unwrap();
            js_on_delegate(&json);
            // Fetch each note
            spawn(async move {
                let mut api_opt = API.with(|a| a.borrow_mut().take());
                if let Some(api) = api_opt.as_mut() {
                    for id in task_ids {
                        send_delegate_msg(api, &DReq::GetNote { task_id: id }).await;
                    }
                }
                API.with(|a| *a.borrow_mut() = api_opt);
            });
        }
        _ => {
            let json = serde_json::to_string(&resp).unwrap();
            js_on_delegate(&json);
        }
    }
}

// ---- Internal: delegate message sending ----
async fn send_delegate_msg(api: &mut WebApi, req: &DReq) {
    let payload = serde_json::to_vec(req).unwrap();
    let app_msg = freenet_stdlib::prelude::ApplicationMessage::new(payload);
    let _ = api.send(ClientRequest::DelegateOp(DelegateRequest::ApplicationMessages {
        key: delegate_key(),
        params: freenet_stdlib::prelude::Parameters::from(vec![]),
        inbound: vec![freenet_stdlib::prelude::InboundDelegateMsg::ApplicationMessage(app_msg)],
    })).await;
}

// ---- Helpers ----
fn bytes_to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

fn hex_to_bytes(h: &str) -> Result<Vec<u8>, std::num::ParseIntError> {
    (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16)).collect()
}

fn hex_short(b: &[u8]) -> String {
    bytes_to_hex(&b[..4])
}

fn now_millis() -> i64 {
    date_now() as i64
}

// ---- spawn helper (wasm-bindgen-futures) ----
fn spawn<F>(fut: F) where F: std::future::Future<Output = ()> + 'static {
    wasm_bindgen_futures::spawn_local(fut);
}
