//! Large catalogues round-trip; unbounded metadata falls back without corrupting the transport.
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
async fn large_parameter_catalogue_round_trips_without_truncation() {
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
    let large = ControlMessage::Parameters {
        request_id: 3,
        parameters: (0..8192).map(parameter).collect(),
    };
    let large_size = serde_json::to_vec(&large).unwrap().len();
    assert!(
        large_size > 1024 * 1024,
        "fixture must exceed the old 1 MiB limit"
    );
    assert!(large_size <= MAX_CONTROL_FRAME_BYTES);
    write_control_response(&mut output, &large).await.unwrap();
    let mut input = output.as_slice();
    assert_eq!(read_control_frame(&mut input).await.unwrap(), large);
    assert!(input.is_empty());
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
    let error = write_control_frame(&mut output, &response)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(
        output.is_empty(),
        "oversized frames must not leave a partial header"
    );
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
