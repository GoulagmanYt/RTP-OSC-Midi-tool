use crossbeam_channel::unbounded;
use osc_midi_bridge::reliable_playback::{
    reliable_playback_metrics_snapshot, reset_reliable_playback_metrics, ReliablePlaybackServer,
};
use serde_json::json;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Duration;

static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn test_guard() -> MutexGuard<'static, ()> {
    TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn send_frame(stream: &mut TcpStream, payload: serde_json::Value) {
    let data = serde_json::to_vec(&payload).expect("serialize frame");
    stream
        .write_all(&(data.len() as u32).to_be_bytes())
        .expect("write frame len");
    stream.write_all(&data).expect("write frame data");
}

fn recv_frame(stream: &mut TcpStream) -> serde_json::Value {
    let mut header = [0u8; 4];
    stream.read_exact(&mut header).expect("read frame len");
    let len = u32::from_be_bytes(header) as usize;
    let mut data = vec![0u8; len];
    stream.read_exact(&mut data).expect("read frame data");
    serde_json::from_slice(&data).expect("json frame")
}

fn connect(server: &ReliablePlaybackServer, read_timeout: Duration) -> TcpStream {
    let stream = TcpStream::connect(server.local_addr()).expect("connect reliable server");
    stream
        .set_read_timeout(Some(read_timeout))
        .expect("set read timeout");
    stream
}

#[test]
fn reliable_server_preserves_duplicates_and_reports_completion() {
    let _guard = test_guard();
    reset_reliable_playback_metrics();
    let (tx, rx) = unbounded();
    let server = ReliablePlaybackServer::start("127.0.0.1:0", tx).expect("start server");
    let mut stream = connect(&server, Duration::from_secs(3));

    send_frame(
        &mut stream,
        json!({
            "type": "prepare",
            "version": 1,
            "sessionId": "session-1",
            "song": "dup.mid",
            "total": 4,
            "events": [
                {"seq": 0, "dueUs": 0, "data": [0x90, 60, 100]},
                {"seq": 1, "dueUs": 0, "data": [0x90, 60, 100]},
                {"seq": 2, "dueUs": 1000, "data": [0xB0, 64, 127]},
                {"seq": 3, "dueUs": 1000, "data": [0x80, 60, 0]}
            ]
        }),
    );
    let prepared = recv_frame(&mut stream);
    assert_eq!(prepared["type"], "prepared");
    assert_eq!(prepared["sessionId"], "session-1");
    assert_eq!(prepared["received"], 4);

    send_frame(
        &mut stream,
        json!({"type": "start", "sessionId": "session-1", "startDelayMs": 5}),
    );
    let completed = recv_frame(&mut stream);
    assert_eq!(completed["type"], "completed");
    assert_eq!(completed["processed"], 4);
    assert_eq!(completed["dropped"], 0);

    let frames: Vec<_> = rx.try_iter().collect();
    assert_eq!(frames.len(), 4);
    assert_eq!(frames[0].data.as_slice(), &[0x90, 60, 100]);
    assert_eq!(frames[1].data.as_slice(), &[0x90, 60, 100]);
    assert_eq!(frames[2].data.as_slice(), &[0xB0, 64, 127]);
    assert_eq!(frames[3].data.as_slice(), &[0x80, 60, 0]);
    assert!(frames
        .iter()
        .all(|frame| &*frame.source == "ReliablePLV:dup.mid"));

    let metrics = reliable_playback_metrics_snapshot();
    assert_eq!(metrics.messages_in, 4);
    assert_eq!(metrics.messages_out, 4);
    assert_eq!(metrics.dropped, 0);
    assert!(metrics.active_session.is_none());

    server.stop();
}

#[test]
fn reliable_server_rejects_missing_or_out_of_order_sequences() {
    let _guard = test_guard();
    reset_reliable_playback_metrics();
    let (tx, _rx) = unbounded();
    let server = ReliablePlaybackServer::start("127.0.0.1:0", tx).expect("start server");
    let mut stream = connect(&server, Duration::from_secs(3));

    send_frame(
        &mut stream,
        json!({
            "type": "prepare",
            "version": 1,
            "sessionId": "bad-seq",
            "song": "bad.mid",
            "total": 2,
            "events": [
                {"seq": 0, "dueUs": 0, "data": [0x90, 60, 100]},
                {"seq": 2, "dueUs": 0, "data": [0x90, 61, 100]}
            ]
        }),
    );
    let error = recv_frame(&mut stream);
    assert_eq!(error["type"], "error");
    assert_eq!(error["sessionId"], "bad-seq");
    assert!(error["message"].as_str().unwrap().contains("seq"));

    let metrics = reliable_playback_metrics_snapshot();
    assert_eq!(metrics.messages_in, 0);
    assert_eq!(metrics.messages_out, 0);

    server.stop();
}

#[test]
fn reliable_server_stop_cancels_session_and_injects_all_notes_off() {
    let _guard = test_guard();
    reset_reliable_playback_metrics();
    let (tx, rx) = unbounded();
    let server = ReliablePlaybackServer::start("127.0.0.1:0", tx).expect("start server");
    let mut stream = connect(&server, Duration::from_secs(3));

    send_frame(
        &mut stream,
        json!({
            "type": "prepare",
            "version": 1,
            "sessionId": "stop-session",
            "song": "long.mid",
            "total": 1,
            "events": [
                {"seq": 0, "dueUs": 60_000_000, "data": [0x90, 60, 100]}
            ]
        }),
    );
    assert_eq!(recv_frame(&mut stream)["type"], "prepared");

    send_frame(
        &mut stream,
        json!({"type": "start", "sessionId": "stop-session", "startDelayMs": 200}),
    );
    send_frame(
        &mut stream,
        json!({"type": "stop", "sessionId": "stop-session"}),
    );
    let stopped = recv_frame(&mut stream);
    assert_eq!(stopped["type"], "stopped");
    assert_eq!(stopped["sessionId"], "stop-session");

    let frames: Vec<_> = rx.try_iter().collect();
    assert_eq!(frames.len(), 32);
    for channel in 0u8..16 {
        let sustain_off = &frames[(channel as usize) * 2];
        let all_notes_off = &frames[(channel as usize) * 2 + 1];
        assert_eq!(sustain_off.data.as_slice(), &[0xB0 | channel, 64, 0]);
        assert_eq!(all_notes_off.data.as_slice(), &[0xB0 | channel, 123, 0]);
        assert_eq!(&*sustain_off.source, "ReliablePLV:long.mid:stop");
        assert_eq!(&*all_notes_off.source, "ReliablePLV:long.mid:stop");
    }

    let metrics = reliable_playback_metrics_snapshot();
    assert_eq!(metrics.messages_in, 1);
    assert_eq!(metrics.messages_out, 0);
    assert_eq!(metrics.dropped, 0);
    assert!(metrics.active_session.is_none());

    server.stop();
}

#[test]
fn reliable_server_processes_50k_preloaded_events_without_drops() {
    let _guard = test_guard();
    reset_reliable_playback_metrics();
    let (tx, rx) = unbounded();
    let server = ReliablePlaybackServer::start("127.0.0.1:0", tx).expect("start server");
    let mut stream = connect(&server, Duration::from_secs(20));
    let total = 50_000usize;
    let events: Vec<_> = (0..total)
        .map(|seq| {
            json!({
                "seq": seq,
                "dueUs": 0,
                "data": [0x90, 21 + (seq % 88), 100]
            })
        })
        .collect();

    send_frame(
        &mut stream,
        json!({
            "type": "prepare",
            "version": 1,
            "sessionId": "stress-50k",
            "song": "stress.mid",
            "total": total,
            "events": events
        }),
    );
    let prepared = recv_frame(&mut stream);
    assert_eq!(prepared["type"], "prepared");
    assert_eq!(prepared["received"], total);

    send_frame(
        &mut stream,
        json!({"type": "start", "sessionId": "stress-50k", "startDelayMs": 1}),
    );
    let completed = recv_frame(&mut stream);
    assert_eq!(completed["type"], "completed");
    assert_eq!(completed["processed"], total);
    assert_eq!(completed["dropped"], 0);

    let frames: Vec<_> = rx.try_iter().collect();
    assert_eq!(frames.len(), total);
    assert_eq!(frames.first().unwrap().data.as_slice(), &[0x90, 21, 100]);
    assert_eq!(
        frames.last().unwrap().data.as_slice(),
        &[0x90, 21 + ((total - 1) % 88) as u8, 100]
    );
    assert!(frames
        .iter()
        .all(|frame| &*frame.source == "ReliablePLV:stress.mid"));

    let metrics = reliable_playback_metrics_snapshot();
    assert_eq!(metrics.messages_in, total as u64);
    assert_eq!(metrics.messages_out, total as u64);
    assert_eq!(metrics.dropped, 0);
    assert!(metrics.active_session.is_none());

    server.stop();
}
