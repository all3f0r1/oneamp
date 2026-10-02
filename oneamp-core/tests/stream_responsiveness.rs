//! The engine loop must keep servicing commands while an HTTP stream is
//! connecting: a silent server used to hold Stop / volume changes for
//! the full 15 s response timeout.

use oneamp_core::{AudioCommand, AudioEngine, AudioEvent};
use std::net::TcpListener;
use std::time::{Duration, Instant};

#[test]
fn commands_are_served_while_a_stream_is_connecting() {
    // Accepts the TCP connection (kernel backlog) but never answers.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/stream", listener.local_addr().unwrap());

    let engine = AudioEngine::new().unwrap();
    engine.send_command(AudioCommand::PlayUrl(url)).unwrap();
    engine.send_command(AudioCommand::SetVolume(0.25)).unwrap();
    engine.send_command(AudioCommand::Stop).unwrap();

    let start = Instant::now();
    let (mut volume_seen, mut stopped_seen) = (false, false);
    while !(volume_seen && stopped_seen) {
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "engine blocked behind the stream connect"
        );
        match engine.try_recv_event() {
            Some(AudioEvent::VolumeUpdated(level, _)) => volume_seen = level == 0.25,
            Some(AudioEvent::Stopped) => stopped_seen = true,
            Some(_) => {}
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    }
}
