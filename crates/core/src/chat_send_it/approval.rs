use super::{full_hash, RunFile, RunPlan};
use crate::worker::hex_encode;
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::fmt;

pub const KIND: &str = "send-it-go";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Approval {
    pub kind: String,
    pub subject_hash: String,
    pub at: String,
    pub peer_uid: u32,
    pub nonce: String,
    pub mac: String,
}

#[derive(Debug, Clone, Copy)]
pub struct Trust<'a> {
    pub key: &'a [u8],
    pub live: Option<&'a str>,
}

impl Trust<'static> {
    pub const NONE: Self = Self {
        key: &[],
        live: None,
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Unsigned,
    Tampered,
    WrongKind,
    WrongPlan,
    Replayed,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Refusal::Unsigned => "not signed by this chat",
            Refusal::Tampered => {
                "the signature does not match this chat (edited, or from before a restart)"
            }
            Refusal::WrongKind => "the record is not a /send-it go approval",
            Refusal::WrongPlan => "the plan changed after it was approved",
            Refusal::Replayed => "the approval is no longer current",
        })
    }
}

pub fn approval_refusal(file: &RunFile, trust: Trust) -> Option<Refusal> {
    let record = file.approved.as_ref()?;
    match verify(Some(record), &file.plan, trust) {
        Err(Refusal::Replayed) if trust.live.is_none() => None,
        Err(refusal) => Some(refusal),
        Ok(()) => None,
    }
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

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    text.as_bytes()
        .chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect()
}

pub fn verify(approval: Option<&Approval>, plan: &RunPlan, trust: Trust) -> Result<(), Refusal> {
    let approval = approval.ok_or(Refusal::Unsigned)?;
    if approval.is_unsigned() {
        return Err(Refusal::Unsigned);
    }
    let given = unhex(&approval.mac).ok_or(Refusal::Tampered)?;
    mac_over(trust.key, approval)
        .map_err(|_| Refusal::Tampered)?
        .verify_slice(&given)
        .map_err(|_| Refusal::Tampered)?;
    if approval.kind != KIND {
        return Err(Refusal::WrongKind);
    }
    if approval.subject_hash != full_hash(plan) {
        return Err(Refusal::WrongPlan);
    }
    if trust.live != Some(approval.nonce.as_str()) {
        return Err(Refusal::Replayed);
    }
    Ok(())
}

impl Approval {
    pub fn is_unsigned(&self) -> bool {
        self.mac.is_empty()
    }

    pub fn seal(self, key: &[u8]) -> Result<Self, String> {
        let mac = mac_over(key, &self)
            .map_err(|error| format!("Could not sign the approval: {error}"))?;
        Ok(Self {
            mac: hex_encode(&mac.finalize().into_bytes()),
            ..self
        })
    }

    pub fn sign(signing: &Signing, plan: &RunPlan) -> Result<Self, String> {
        Self {
            kind: KIND.to_string(),
            subject_hash: full_hash(plan),
            at: signing.at.to_string(),
            peer_uid: signing.peer_uid,
            nonce: signing.nonce.to_string(),
            mac: String::new(),
        }
        .seal(signing.key)
    }
}
