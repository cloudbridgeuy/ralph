use super::{full_hash, RunPlan};
use crate::worker::hex_encode;
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

pub const KIND: &str = "send-it-go";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Approval {
    pub kind: String,
    pub subject_hash: String,
    pub at: String,
    pub peer_uid: u32,
    pub nonce: String,
    pub mac: String,
}

pub struct Signing<'a> {
    pub key: &'a [u8],
    pub at: &'a str,
    pub peer_uid: u32,
    pub nonce: &'a str,
}

fn mac_over(key: &[u8], record: &Approval) -> Result<Hmac<Sha256>, String> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).map_err(|error| error.to_string())?;
    let uid = record.peer_uid.to_be_bytes();
    for field in [
        record.kind.as_bytes(),
        record.subject_hash.as_bytes(),
        record.at.as_bytes(),
        uid.as_slice(),
        record.nonce.as_bytes(),
    ] {
        mac.update(&(field.len() as u64).to_be_bytes());
        mac.update(field);
    }
    Ok(mac)
}

impl Approval {
    pub fn sign(signing: &Signing, plan: &RunPlan) -> Result<Self, String> {
        let record = Self {
            kind: KIND.to_string(),
            subject_hash: full_hash(plan),
            at: signing.at.to_string(),
            peer_uid: signing.peer_uid,
            nonce: signing.nonce.to_string(),
            mac: String::new(),
        };
        let mac = mac_over(signing.key, &record)
            .map_err(|error| format!("Could not sign the approval: {error}"))?;
        Ok(Self {
            mac: hex_encode(&mac.finalize().into_bytes()),
            ..record
        })
    }
}
