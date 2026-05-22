// SPDX-License-Identifier: Apache-2.0
//! B-2: Layer 2 DictSync integration tests.
//! Ensures rejected or unknown dynamic IDs cannot silently decode incorrectly.

use laplace_probe::domain::wire::{
    DictSyncError, DictSyncMessage, DictSyncSession, DYNAMIC_ID_MAX, DYNAMIC_ID_MIN,
};

#[test]
fn test_dict_sync_ack_flow_confirms_id() {
    let mut session = DictSyncSession::new();
    session.register_pending(DYNAMIC_ID_MIN, "GET /v1/orders".to_string());
    session.handle(DictSyncMessage::Ack {
        confirmed_id: DYNAMIC_ID_MIN,
    });

    assert_eq!(session.check_id(DYNAMIC_ID_MIN), Ok("GET /v1/orders"));
}

#[test]
fn test_dict_sync_reject_blocks_id_usage() {
    let mut session = DictSyncSession::new();
    session.register_pending(DYNAMIC_ID_MIN, "GET /v1/orders".to_string());
    session.handle(DictSyncMessage::Reject {
        rejected_id: DYNAMIC_ID_MIN,
    });

    assert_eq!(
        session.check_id(DYNAMIC_ID_MIN),
        Err(DictSyncError::RejectedId(DYNAMIC_ID_MIN))
    );
}

#[test]
fn test_dict_sync_unknown_id_returns_error() {
    let session = DictSyncSession::new();

    assert_eq!(
        session.check_id(DYNAMIC_ID_MIN),
        Err(DictSyncError::UnknownId(DYNAMIC_ID_MIN))
    );
}

#[test]
fn test_dict_sync_message_serialization_roundtrip() {
    let messages = [
        DictSyncMessage::Propose {
            proposed_id: DYNAMIC_ID_MIN,
            token: "POST /sync".to_string(),
        },
        DictSyncMessage::Ack {
            confirmed_id: DYNAMIC_ID_MIN + 1,
        },
        DictSyncMessage::Reject {
            rejected_id: DYNAMIC_ID_MIN + 2,
        },
    ];

    for message in messages {
        let bytes = message.to_bytes();
        let (decoded, consumed) = DictSyncMessage::from_bytes(&bytes).unwrap();
        assert_eq!(decoded, message);
        assert_eq!(consumed, bytes.len());
    }
}

#[test]
fn test_dict_sync_id_range_check() {
    let session = DictSyncSession::new();

    assert_eq!(
        session.check_id(DYNAMIC_ID_MIN - 1),
        Err(DictSyncError::UnknownId(DYNAMIC_ID_MIN - 1))
    );
    assert_eq!(
        session.check_id(DYNAMIC_ID_MAX + 1),
        Err(DictSyncError::UnknownId(DYNAMIC_ID_MAX + 1))
    );
}
