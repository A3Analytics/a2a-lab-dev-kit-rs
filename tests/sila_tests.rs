#![cfg(feature = "sila2")]

use a2a_lab_dev_kit::TaskState;
use a2a_lab_dev_kit::sila::{
    MAX_CHUNK, certificate_accepted, chunk_binary, execution_state, parse_discovery,
};

#[test]
fn maps_execution_discovery_identity_and_binary_chunks() {
    assert_eq!(execution_state(0).unwrap(), TaskState::Submitted);
    assert_eq!(execution_state(1).unwrap(), TaskState::Working);
    assert_eq!(execution_state(2).unwrap(), TaskState::Completed);
    assert_eq!(execution_state(3).unwrap(), TaskState::Failed);
    assert!(execution_state(9).is_err());

    let endpoint = parse_discovery("_sila._tcp.local.", "lab.local", 50052, "server-1").unwrap();
    assert_eq!(endpoint.server_uuid, "server-1");
    assert!(parse_discovery("_http._tcp.local.", "lab.local", 80, "server-1").is_err());
    assert!(certificate_accepted("SiLA2", "server-1", "server-1").is_ok());
    assert!(certificate_accepted("SiLA2", "server-1", "other").is_err());

    let payload = vec![1_u8; MAX_CHUNK + 1];
    let chunks = chunk_binary(&payload);
    assert_eq!(chunks.len(), 2);
    assert!(chunks.iter().all(|chunk| chunk.len() <= MAX_CHUNK));
}
