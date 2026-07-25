use freenet_stdlib::prelude::*;
use serde::{Deserialize, Serialize};

struct NotesDelegate;

/// All requests the delegate handles. Tagged with `type` so the UI
/// serializes as `{ "type": "SaveNote", ... }` etc.
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
enum Request {
    SaveNote { task_id: u64, text: String },
    GetNote { task_id: u64 },
    ListNotes,
    DeleteNote { task_id: u64 },
    SaveIdentity { private_key_hex: String, public_key_hex: String },
    GetIdentity,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
enum Response {
    Saved { task_id: u64 },
    Note { task_id: u64, text: Option<String> },
    NotesList { task_ids: Vec<u64> },
    Deleted { task_id: u64 },
    Identity { private_key_hex: Option<String>, public_key_hex: Option<String> },
    IdentitySaved,
    Error { message: String },
}

fn instance_prefix(origin: &Option<MessageOrigin>) -> String {
    match origin {
        Some(MessageOrigin::WebApp(id)) => format!("note:{}", id),
        _ => "note:_:".to_string(),
    }
}

fn note_key(prefix: &str, task_id: u64) -> Vec<u8> {
    format!("{prefix}{task_id}").into_bytes()
}

const IDENTITY_KEY: &[u8] = b"identity";

fn reply(resp: Response) -> Vec<OutboundDelegateMsg> {
    let payload = serde_json::to_vec(&resp).unwrap_or_default();
    vec![OutboundDelegateMsg::ApplicationMessage(ApplicationMessage::new(
        payload,
    ))]
}

#[delegate]
impl DelegateInterface for NotesDelegate {
    fn process(
        ctx: &mut DelegateCtx,
        _parameters: Parameters<'static>,
        origin: Option<MessageOrigin>,
        message: InboundDelegateMsg,
    ) -> Result<Vec<OutboundDelegateMsg>, DelegateError> {
        let app_msg = match message {
            InboundDelegateMsg::ApplicationMessage(m) => m,
            _ => return Ok(vec![]),
        };

        let req: Request = match serde_json::from_slice(&app_msg.payload) {
            Ok(r) => r,
            Err(e) => return Ok(reply(Response::Error { message: e.to_string() })),
        };

        let prefix = instance_prefix(&origin);

        match req {
            Request::SaveNote { task_id, text } => {
                let key = note_key(&prefix, task_id);
                if ctx.set_secret(&key, text.as_bytes()) {
                    Ok(reply(Response::Saved { task_id }))
                } else {
                    Ok(reply(Response::Error {
                        message: "set_secret failed".into(),
                    }))
                }
            }
            Request::GetNote { task_id } => {
                let key = note_key(&prefix, task_id);
                let text = ctx.get_secret(&key).and_then(|b| String::from_utf8(b).ok());
                Ok(reply(Response::Note { task_id, text }))
            }
            Request::DeleteNote { task_id } => {
                let key = note_key(&prefix, task_id);
                let _ = ctx.remove_secret(&key);
                Ok(reply(Response::Deleted { task_id }))
            }
            Request::ListNotes => {
                let pfx = prefix.as_bytes().to_vec();
                let keys = ctx.list_secrets(&pfx);
                let mut task_ids = Vec::new();
                for k in keys {
                    if let Ok(s) = std::str::from_utf8(&k) {
                        if let Some(suffix) = s.strip_prefix(&format!("{prefix}")) {
                            if let Ok(id) = suffix.parse::<u64>() {
                                task_ids.push(id);
                            }
                        }
                    }
                }
                task_ids.sort_unstable();
                Ok(reply(Response::NotesList { task_ids }))
            }
            Request::SaveIdentity {
                private_key_hex,
                public_key_hex,
            } => {
                // Store as JSON so both keys round-trip in one secret.
                let blob = serde_json::to_vec(&IdentityBlob {
                    private_key_hex,
                    public_key_hex,
                })
                .unwrap_or_default();
                if ctx.set_secret(IDENTITY_KEY, &blob) {
                    Ok(reply(Response::IdentitySaved))
                } else {
                    Ok(reply(Response::Error {
                        message: "set_secret failed for identity".into(),
                    }))
                }
            }
            Request::GetIdentity => {
                let blob = ctx.get_secret(IDENTITY_KEY);
                let identity = blob.and_then(|b| {
                    serde_json::from_slice::<IdentityBlob>(&b)
                        .ok()
                        .map(|i| (i.private_key_hex, i.public_key_hex))
                });
                match identity {
                    Some((priv_hex, pub_hex)) => Ok(reply(Response::Identity {
                        private_key_hex: Some(priv_hex),
                        public_key_hex: Some(pub_hex),
                    })),
                    None => Ok(reply(Response::Identity {
                        private_key_hex: None,
                        public_key_hex: None,
                    })),
                }
            }
        }
    }
}

#[derive(Serialize, Deserialize)]
struct IdentityBlob {
    private_key_hex: String,
    public_key_hex: String,
}
