use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const TASK_TAG: &[u8] = b"todo-task\x00";
const TOGGLE_TAG: &[u8] = b"todo-toggle\x00";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SignedTask {
    pub id: u64,
    pub text: String,
    pub created_at: i64,
    pub author: VerifyingKey,
    pub signature: Signature,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SignedToggle {
    pub task_id: u64,
    pub done: bool,
    pub ts: i64,
    pub author: VerifyingKey,
    pub signature: Signature,
}

#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Eq, Debug)]
pub struct TodoState {
    pub tasks: BTreeMap<u64, SignedTask>,
    pub toggles: BTreeMap<u64, SignedToggle>,
}

impl TodoState {
    pub fn merge(&mut self, other: TodoState) {
        for (id, task) in other.tasks {
            if !task.verify().unwrap_or(false) {
                continue;
            }
            self.tasks.entry(id).or_insert(task);
        }
        for (task_id, toggle) in other.toggles {
            if !toggle.verify().unwrap_or(false) {
                continue;
            }
            match self.toggles.get(&task_id) {
                Some(existing) => {
                    if toggle.ts > existing.ts
                        || (toggle.ts == existing.ts && toggle.signature.to_bytes() > existing.signature.to_bytes())
                    {
                        self.toggles.insert(task_id, toggle);
                    }
                }
                None => {
                    self.toggles.insert(task_id, toggle);
                }
            }
        }
    }

    pub fn validate(&self) -> bool {
        self.tasks.values().all(|t| t.verify().unwrap_or(false))
            && self.toggles.values().all(|t| t.verify().unwrap_or(false))
    }
}

impl SignedTask {
    pub fn signing_bytes(&self) -> Vec<u8> {
        let text_bytes = self.text.as_bytes();
        let mut buf = Vec::with_capacity(TASK_TAG.len() + 8 + 4 + text_bytes.len() + 8);
        buf.extend_from_slice(TASK_TAG);
        buf.extend_from_slice(&self.id.to_le_bytes());
        buf.extend_from_slice(&(text_bytes.len() as u32).to_le_bytes());
        buf.extend_from_slice(text_bytes);
        buf.extend_from_slice(&self.created_at.to_le_bytes());
        buf
    }

    pub fn verify(&self) -> Result<bool, String> {
        let sig = self.signature;
        Ok(self.author.verify_strict(&self.signing_bytes(), &sig).is_ok())
    }
}

impl SignedToggle {
    pub fn signing_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(TOGGLE_TAG.len() + 8 + 1 + 8);
        buf.extend_from_slice(TOGGLE_TAG);
        buf.extend_from_slice(&self.task_id.to_le_bytes());
        buf.push(self.done as u8);
        buf.extend_from_slice(&self.ts.to_le_bytes());
        buf
    }

    pub fn verify(&self) -> Result<bool, String> {
        let sig = self.signature;
        Ok(self.author.verify_strict(&self.signing_bytes(), &sig).is_ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn keypair(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn mk_task(sk: &SigningKey, id: u64, text: &str, ts: i64) -> SignedTask {
        let author = sk.verifying_key();
        let unsig = SignedTask {
            id,
            text: text.into(),
            created_at: ts,
            author,
            signature: Signature::from_bytes(&[0u8; 64]),
        };
        let signature = sk.sign(&unsig.signing_bytes());
        SignedTask { signature, ..unsig }
    }

    fn mk_toggle(sk: &SigningKey, task_id: u64, done: bool, ts: i64) -> SignedToggle {
        let author = sk.verifying_key();
        let unsig = SignedToggle {
            task_id,
            done,
            ts,
            author,
            signature: Signature::from_bytes(&[0u8; 64]),
        };
        let signature = sk.sign(&unsig.signing_bytes());
        SignedToggle { signature, ..unsig }
    }

    #[test]
    fn task_sign_verify_roundtrip() {
        let sk = keypair(7);
        let t = mk_task(&sk, 1, "hello", 1000);
        assert!(t.verify().unwrap());
    }

    #[test]
    fn merge_is_commutative() {
        let sk = keypair(7);
        let mut a = TodoState::default();
        a.merge(TodoState {
            tasks: [(1, mk_task(&sk, 1, "a", 1))].into_iter().collect(),
            toggles: [(1, mk_toggle(&sk, 1, true, 5))].into_iter().collect(),
        });
        let mut b = TodoState::default();
        b.merge(TodoState {
            tasks: [(2, mk_task(&sk, 2, "b", 2))].into_iter().collect(),
            toggles: [(1, mk_toggle(&sk, 1, true, 9))].into_iter().collect(),
        });

        let mut ab = a.clone();
        ab.merge(b.clone());
        let mut ba = b.clone();
        ba.merge(a.clone());
        assert_eq!(ab, ba);
        assert_eq!(ab.toggles.get(&1).unwrap().ts, 9);
    }

    #[test]
    fn invalid_signature_rejected_on_merge() {
        let mut bad = mk_task(&keypair(7), 1, "x", 1);
        let mut sig_bytes = bad.signature.to_bytes();
        sig_bytes[0] ^= 1;
        bad.signature = Signature::from_bytes(&sig_bytes);
        let mut state = TodoState::default();
        state.merge(TodoState {
            tasks: [(1, bad)].into_iter().collect(),
            toggles: BTreeMap::new(),
        });
        assert!(state.tasks.is_empty());
    }
}
