//! What this crate's wiring actually puts on the wire, read by a listener standing in for the
//! viewer.
//!
//! The unit tests beside this one cover the two pure decisions — which address is accepted and which
//! tab a target is filed under. This covers the part neither can reach: that the layer, the client
//! and the protocol between them turn a `tracing` event into the record a viewer will draw. A crate
//! whose whole job is to reach another program is one where everything can be right in isolation and
//! wrong together.
//!
//! **Loopback and an ephemeral port**, which is `No test binds a non-loopback address` in
//! `CONTRIBUTING.md`: a test binary's path carries a build hash, so one that bound an outward
//! address would raise a fresh Windows firewall prompt on every rebuild and leave a dead rule behind
//! each time.
//!
//! **A scoped subscriber and not a global one.** `set_global_default` takes for the life of the
//! process, so a second test here would find it taken; `with_default` holds only for the closure,
//! which is also what lets this file assert on a stream it can see the end of.

use std::io::Read as _;
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::Once;
use std::time::Duration;

use tracing_subscriber::layer::SubscriberExt as _;

/// The banner every connection opens with.
const CMD_BANNER: u8 = 99;
/// One log record.
const CMD_LOG: u8 = 0;

/// A viewer that is listening, and what it heard.
fn collect(app_name: &str, emit: impl FnOnce()) -> (String, Vec<serde_json::Value>) {
    let listener =
        TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).expect("bind loopback");
    let address = listener
        .local_addr()
        .expect("read back the port")
        .to_string();

    let viewer = km_ecapplog::EcAppLog::open(&address, app_name);
    let subscriber = tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new("trace"))
        .with(viewer.layer());

    let accepted = std::thread::spawn(move || listener.accept().map(|(stream, _)| stream));

    tracing::subscriber::with_default(subscriber, emit);
    // Drains what was queued, which is what makes the read below terminate at a known point rather
    // than at a timeout.
    viewer.flush();

    let stream = accepted
        .join()
        .expect("the listener thread")
        .expect("a connection");
    read_frames(stream)
}

/// Reads frames until the client stops talking, returning the banner's app name and the records.
fn read_frames(mut stream: TcpStream) -> (String, Vec<serde_json::Value>) {
    // The client has already been flushed, so everything it means to say is in the socket. The
    // timeout is the backstop for a fault rather than the ordinary path.
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("set a read timeout");

    let mut app_name = String::new();
    let mut records = Vec::new();
    let mut header = [0_u8; 5];
    while stream.read_exact(&mut header).is_ok() {
        let length = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
        let mut payload = vec![0_u8; length];
        stream.read_exact(&mut payload).expect("a whole payload");
        let payload = String::from_utf8(payload).expect("utf-8");
        match header[0] {
            CMD_BANNER => {
                app_name = payload
                    .strip_prefix("ECAPPLOG ")
                    .expect("the banner names the protocol first")
                    .to_owned();
            }
            CMD_LOG => records.push(serde_json::from_str(&payload).expect("a JSON record")),
            other => panic!("unknown command {other}"),
        }
    }
    (app_name, records)
}

/// An event arrives as the record a viewer draws: the app's name, this crate's tab, the level, and
/// the message apart from the fields.
#[test]
fn an_event_arrives_as_a_record() {
    let (app_name, records) = collect("KaraokeMachine", || {
        tracing::info!(port = 8177, "listening");
    });

    assert_eq!(app_name, "KaraokeMachine");
    assert_eq!(records.len(), 1, "one event, one record");
    let record = &records[0];

    // `INFORMATION`, not `INFO`: the protocol spells the informational one out.
    assert_eq!(record["priority"], "INFORMATION");
    assert_eq!(record["message"], "listening");
    // The crate this test binary is, which is what `category` derives from a target. Filed by the
    // crate rather than the module — see the note on that function.
    assert_eq!(record["category"], "wire");

    // The protocol carries one flat message, so a field rides in `source` as JSON. That is what the
    // viewer pretty-prints in its details pane, and it is the whole of why `tracing` fits a wire
    // with no notion of structure.
    let source: serde_json::Value =
        serde_json::from_str(record["source"].as_str().expect("a source string"))
            .expect("the source is JSON");
    assert_eq!(source["fields"]["port"], 8177);
}

/// **The timestamp is the viewer's own format and not RFC 3339**, which is the one part of this
/// protocol that fails silently: a `Z` or an offset is rejected, and the server then substitutes its
/// own arrival time rather than reporting anything — so a client getting this wrong looks like it
/// works while every row shows a time nobody sent.
#[test]
fn a_timestamp_is_the_format_the_viewer_parses() {
    let (_, records) = collect("KaraokeMachine", || tracing::warn!("careful"));

    let time = records[0]["time"].as_str().expect("a time string");
    // `yyyy-MM-ddThh:mm:ss.zzz`: exactly three fractional digits, no zone marker at all.
    assert_eq!(time.len(), 23, "{time} is not the 23-character form");
    assert_eq!(&time[4..5], "-");
    assert_eq!(&time[10..11], "T");
    assert_eq!(&time[19..20], ".");
    assert!(
        !time.ends_with('Z') && !time.contains('+'),
        "{time} carries a zone the viewer refuses"
    );
    assert!(
        time[20..].chars().all(|c| c.is_ascii_digit()),
        "{time} does not end in three digits"
    );
}

/// Every level `tracing` has reaches the viewer as the priority it means.
///
/// `NOTICE`, `CRITICAL` and `FATAL` are unreachable through a layer — `tracing` has five levels and
/// the protocol has eight — which is a property of the mapping rather than a gap: nothing here has
/// ever wanted a level between `warn` and `error`.
#[test]
fn every_level_maps_to_a_priority() {
    let (_, records) = collect("KaraokeMachine", || {
        tracing::trace!("t");
        tracing::debug!("d");
        tracing::info!("i");
        tracing::warn!("w");
        tracing::error!("e");
    });

    let priorities: Vec<_> = records
        .iter()
        .map(|record| record["priority"].as_str().expect("a priority"))
        .collect();
    assert_eq!(
        priorities,
        ["TRACE", "DEBUG", "INFORMATION", "WARNING", "ERROR"]
    );
}

/// A span's fields travel with the events inside it, which is what makes a request's log readable
/// once several are interleaved.
#[test]
fn a_span_travels_with_the_events_inside_it() {
    let (_, records) = collect("KaraokeMachine", || {
        let span = tracing::info_span!("play", song_id = "abc123");
        let _entered = span.enter();
        tracing::info!("started");
    });

    let source: serde_json::Value =
        serde_json::from_str(records[0]["source"].as_str().expect("a source string"))
            .expect("the source is JSON");
    assert_eq!(source["spans"][0]["name"], "play");
    assert_eq!(source["spans"][0]["fields"]["song_id"], "abc123");
}

/// A dependency that speaks the `log` facade gets its own tab, and not one called `log`.
///
/// **The bridge is what this is about.** `tracing-log` carries a `log` record into `tracing` under a
/// target of its own, with the record's real one riding as a field, so a layer that reads the target
/// off the metadata files every such dependency together. The crates that reach the viewer this way
/// are the ones a person is least able to guess the origin of — `mdns_sd` and whatever else the tree
/// carries — which is exactly when a tab per crate earns its keep.
///
/// `LogTracer` takes the global logger for the life of this binary, where the subscriber these tests
/// install is scoped to a closure. That is why it is behind a `Once` and why it costs the other
/// tests nothing: the bridge dispatches to whichever subscriber is current.
#[test]
fn a_log_facade_record_is_filed_by_its_crate() {
    static BRIDGE: Once = Once::new();
    BRIDGE.call_once(|| {
        tracing_log::LogTracer::init().expect("nothing else has taken the global logger");
    });

    let (_, records) = collect("KaraokeMachine", || {
        log::debug!(target: "mdns_sd::service_daemon", "sending query");
    });

    let record = &records[0];
    assert_eq!(record["category"], "mdns_sd");
    assert_eq!(record["message"], "sending query");
    assert_eq!(record["priority"], "DEBUG");
    // The GUI draws `category [original_category]`, so the module a line came from is a click away
    // from the tab its crate owns. The target above is borrowed from a real dependency and the
    // module path is this file's, the `log` macro taking that from where it is written; in a run
    // they are `mdns_sd` and `mdns_sd::service_daemon`.
    assert_eq!(record["original_category"], module_path!());

    // The four fields the bridge carries its metadata in are the tab, the module and the call site
    // by now; a copy of each in the details pane would be the same facts twice.
    let source: serde_json::Value =
        serde_json::from_str(record["source"].as_str().expect("a source string"))
            .expect("the source is JSON");
    assert!(
        source["at"]
            .as_str()
            .expect("a call site")
            .contains("wire.rs:"),
        "the call site is not the one the macro recorded"
    );
    assert!(
        source.get("fields").is_none(),
        "the bridge's own fields reached the details pane"
    );
}

/// **A viewer that is not there costs nothing**, which is the property that lets a program be
/// started first and attached to afterwards. Nothing here blocks, nothing fails, and the run carries
/// on with its other destinations.
#[test]
fn an_absent_viewer_is_not_an_error() {
    // Bound and dropped, so the port is one nothing is listening on. A port nobody ever held would
    // do as well and could collide with something that arrived meanwhile.
    let listener =
        TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).expect("bind loopback");
    let address = listener
        .local_addr()
        .expect("read back the port")
        .to_string();
    drop(listener);

    let viewer = km_ecapplog::EcAppLog::open(&address, "KaraokeMachine");
    let subscriber = tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new("trace"))
        .with(viewer.layer());
    tracing::subscriber::with_default(subscriber, || {
        tracing::info!("nobody is reading this");
    });
    viewer.flush();
}
