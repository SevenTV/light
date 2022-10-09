use std::collections::HashMap;

use anyhow::Result;
use bitflags::bitflags;

mod dns;

pub mod supervisor;
pub mod types;

bitflags! {
    pub struct TunnelFeatures: u32 {
        const SERIALIZED_HEADERS = 1 << 0;
        const QUICK_RECONNECT = 1 << 1;
        const ALLOW_REMOTE_CONFIG = 1 << 2;
        const DATAGRAM_V2 = 1 << 3;
        const POST_QUANTUM = 1 << 4;

        const DEFAULT = Self::ALLOW_REMOTE_CONFIG.bits | Self::SERIALIZED_HEADERS.bits | Self::DATAGRAM_V2.bits;
    }
}

impl TunnelFeatures {
    pub fn to_vec(&self) -> Vec<String> {
        let mut features = Vec::new();

        if self.contains(Self::SERIALIZED_HEADERS) {
            features.push("serialized_headers".to_string());
        }

        if self.contains(Self::QUICK_RECONNECT) {
            features.push("quick_reconnect".to_string());
        }

        if self.contains(Self::ALLOW_REMOTE_CONFIG) {
            features.push("allow_remote_config".to_string());
        }

        if self.contains(Self::DATAGRAM_V2) {
            features.push("datagram_v2".to_string());
        }

        if self.contains(Self::POST_QUANTUM) {
            features.push("post_quantum".to_string());
        }

        features
    }
}

pub struct ClientInfo {
    pub client_id: uuid::Uuid, // this is required for capnp
    pub features: TunnelFeatures,
    pub version: String,
    pub arch: String,
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub enum Protocol {
    NONE,
    QUIC,
    HTTP2,
}

pub struct TLSSettings {
    pub server_name: String,
    pub next_protos: Vec<String>,
}

impl Protocol {
    pub fn fallback(&self) -> Self {
        match self {
            Protocol::QUIC => Protocol::HTTP2,
            Protocol::HTTP2 => Protocol::NONE,
            Protocol::NONE => Protocol::NONE,
        }
    }

    pub fn tls_settings(&self) -> TLSSettings {
        match self {
            Protocol::QUIC => TLSSettings {
                server_name: "quic.cftunnel.com".to_string(),
                next_protos: vec!["argotunnel".to_string()],
            },
            Protocol::HTTP2 => TLSSettings {
                server_name: "h2.cftunnel.com".to_string(),
                next_protos: vec![],
            },
            Protocol::NONE => panic!("no tls settings for protocol NONE"),
        }
    }
}
