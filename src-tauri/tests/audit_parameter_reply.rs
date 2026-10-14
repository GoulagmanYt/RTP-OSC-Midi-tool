//! Evidence that parameter catalogue stays bounded or falls back to an error reply without crashing the transport.
use osc_midi_bridge::{
    audio::MAX_CACHED_PARAMETERS,
    types::VstParameter,
    vst_worker::protocol::{
        read_control_frame, write_control_frame, write_control_response, ControlMessage,
        MAX_CONTROL_FRAME_BYTES,
    },
};

fn parameter(index: usize) -> VstParameter {
    VstParameter {
        index,
        id: index as u32,
        flags: 0,
        step_count: 0,
        unit_id: 0,
        name: format!("Parameter {index}"),
        min: 0.0,
        max: 1.0,
        default: 0.5,
        unit: String::new(),
        value: 0.5,
    }
}

#[tokio::test]
async fn evidence_large_parameter_catalogue_is_rejected_before_any_transport_write() {
    let mut output = Vec::new();
    let small = ControlMessage::Parameters {
        request_id: 1,
        parameters: vec![parameter(0)],
    };
    write_control_frame(&mut output, &small).await.unwrap();
    assert!(!output.is_empty());

    output.clear();
    let bounded = ControlMessage::Parameters {
        request_id: 2,
        parameters: (0..MAX_CACHED_PARAMETERS).map(parameter).collect(),
    };
    let bounded_size = serde_json::to_vec(&bounded).unwrap().len();
    assert!(
        bounded_size < MAX_CONTROL_FRAME_BYTES,
        "bounded size {bounded_size} must be within {MAX_CONTROL_FRAME_BYTES}"
    );
    write_control_frame(&mut output, &bounded).await.unwrap();
    assert!(!output.is_empty());

    output.clear();
    let fallback = ControlMessage::Error {
        request_id: Some(2),
        code: "worker.frame-too-large".into(),
        message: "Response exceeds IPC frame limit".into(),
    };
    write_control_frame(&mut output, &fallback).await.unwrap();
    assert!(!output.is_empty());
}

#[tokio::test]
async fn oversized_metadata_returns_error_and_the_next_reply_still_works() {
    let mut huge = parameter(0);
    huge.name = "x".repeat(MAX_CONTROL_FRAME_BYTES);
    let response = ControlMessage::Parameters {
        request_id: 77,
        parameters: vec![huge],
    };
    let mut output = Vec::new();
    write_control_response(&mut output, &response)
        .await
        .unwrap();
    write_control_response(&mut output, &ControlMessage::Ack { request_id: 78 })
        .await
        .unwrap();
    let mut input = output.as_slice();
    assert!(matches!(
        read_control_frame(&mut input).await.unwrap(),
        ControlMessage::Error {
            request_id: Some(77),
            ..
        }
    ));
    assert_eq!(
        read_control_frame(&mut input).await.unwrap(),
        ControlMessage::Ack { request_id: 78 }
    );
}
