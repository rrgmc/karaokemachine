//! Serves the API over an in-memory machine, so the dev remote can be driven in a browser.
//!
//! ```sh
//! cargo run -p km-api --example dev_server --features testing
//! cargo run -p km-api --example dev_server --features testing -- --lan --password hunter2
//! cargo run -p km-api --example dev_server --features testing -- --port 8277
//! ```
//!
//! Why this exists: M6's exit criteria include "drive every feature from the dev remote in a
//! browser" and "discovery seen from a phone", and both need a *running server* — which `km-app`
//! will not be until M7. Rather than leave those unverifiable for a whole milestone, this example
//! wires the real router to the `testing` machine. Everything is real except the engine and the
//! catalog: real routing, real ACL, real admin tokens, real WebSocket, real mDNS advertisement.
//!
//! It is an example and not a binary on purpose. Nothing ships it.

use std::path::PathBuf;

use km_api::testing::TestMachine;
use km_api::{ApiConfig, ApiState, bind, bind_with};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| format!("info,{}=debug", km_api::LOG_TARGET).into()),
        )
        .init();

    let mut lan = false;
    let mut password: Option<String> = None;
    let mut songs = 40_u32;
    let mut port = km_api::DEFAULT_PORT;
    let mut stream_dir: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            // Bind 0.0.0.0 so a phone can reach it — the configuration the connect panel and the
            // mDNS advertisement actually have to work in.
            "--lan" => lan = true,
            "--password" => password = args.next(),
            "--songs" => {
                songs = args
                    .next()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(songs)
            }
            // **Unlike `--songs`, a value that will not read is fatal here.** A mistyped song count
            // gives a differently-sized catalog and you notice; a mistyped port silently binds
            // 8177, which is the collision the flag was typed to avoid, and the machine it collides
            // with is the one that then looks broken.
            "--port" => {
                let value = args.next().unwrap_or_default();
                port = value.parse().map_err(|_| {
                    format!("could not read '{value}' as a port — try `--port 8277`")
                })?;
            }
            // A directory an encoder is writing HLS into, served at `/stream/` with the watch page
            // above it. **Nothing here produces one** — this example has an in-memory machine and
            // no screen to draw — so it is pointed at a directory something else filled, which is
            // what makes the serving side testable in a browser on its own.
            "--stream-dir" => {
                stream_dir = args.next().map(PathBuf::from);
            }
            "--help" | "-h" => {
                println!(
                    "usage: dev_server [--lan] [--password <password>] [--songs <n>] \
                     [--port <n>] [--stream-dir <dir>]\n\n\
                     Serves the API over an in-memory machine, with the dev remote at /dev/.\n\
                     With --stream-dir, also serves that directory at /stream/ and the watch \
                     page at /watch/."
                );
                return Ok(());
            }
            other => {
                eprintln!("unknown argument '{other}' — try --help");
                std::process::exit(2);
            }
        }
    }

    // The dev remote lives beside this crate in the repository. Resolved from the manifest
    // directory so it works whatever the shell's working directory is.
    let dev_remote = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../tools/dev/remote");

    let mut config = ApiConfig::default().with_dev_remote(dev_remote.clone());
    config.machine_name = "Dev Machine".to_owned();
    config.bind.set_port(port);
    if lan {
        config = config.on_all_interfaces(port);
    }
    if let Some(password) = &password {
        // `password_hash::Error` does not implement `std::error::Error`, so it cannot ride the `?`.
        config = config
            .with_password(password)
            .map_err(|error| format!("could not hash the password: {error}"))?;
    }

    let machine = TestMachine::with_catalog(songs).shared();
    let state = ApiState::from_machine(machine, config);
    let listening = match &stream_dir {
        Some(dir) => {
            let extras = km_api::Extras {
                stream: Some(km_api::watch::router(dir)),
                ..Default::default()
            };
            bind_with(state, extras).await?
        }
        None => bind(state).await?,
    };

    println!("\nlistening on   {}", listening.local_addr);
    match listening.connect.primary_url() {
        Some(url) => {
            println!("dev remote     {url}/dev/");
            for alternate in listening.connect.alternate_urls() {
                println!("also reachable {alternate}/dev/");
            }
        }
        None => println!("dev remote     no reachable address — try --lan"),
    }
    if !listening.connect.reachable {
        // The honest failure states are the interesting part of the connect panel, so print the
        // reason rather than pretending.
        println!("reachable      no: {:?}", listening.connect.problem);
    }
    // What was asked for, not what has happened: this banner prints before `serve`, and the
    // advertisement is now started by a task inside it. The log says when it begins.
    // The environment is read beside the setting because a banner that named only the setting would
    // promise an advertisement a declined run is never going to make.
    println!(
        "advertising    {}",
        if km_api::discover::mdns_declined() {
            "off — declined by KM_NO_MDNS"
        } else if listening.state().config().advertise_mdns {
            "mDNS on — the log says when it starts"
        } else {
            "off"
        }
    );
    println!(
        "admin          {}",
        password.map_or_else(
            || "off — every route is public, admin marks dormant".to_owned(),
            |_| "password set".to_owned()
        )
    );
    println!("\nCtrl-C to stop.\n");

    listening
        .serve_with_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            println!("\nstopping.");
        })
        .await?;
    Ok(())
}
