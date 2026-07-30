use std::fs;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use textora_sync::{ApiKey, LoopbackEndpoint, SyncthingClient};

struct SyncthingProcess {
    child: Child,
}

impl Drop for SyncthingProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
#[ignore = "requires a Syncthing 2.1.x binary via SYNCTHING_BIN"]
fn real_syncthing_probe_uses_an_isolated_home() {
    let Some(binary) = std::env::var_os("SYNCTHING_BIN") else {
        eprintln!("skipping: SYNCTHING_BIN is not set");
        return;
    };

    let home = isolated_home();
    fs::create_dir_all(&home).expect("isolated Syncthing home should be created");
    let gui_port = reserve_port();
    let gui_address = format!("127.0.0.1:{gui_port}");
    let child = Command::new(binary)
        .arg("--no-browser")
        .arg("--no-default-folder")
        .arg(format!("--home={}", home.display()))
        .arg(format!("--gui-address={gui_address}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("SYNCTHING_BIN should start");
    let _process = SyncthingProcess { child };
    let endpoint_text = format!("http://{gui_address}");
    let endpoint = LoopbackEndpoint::parse(&endpoint_text).expect("GUI endpoint should parse");
    let deadline = Instant::now() + Duration::from_secs(15);

    while Instant::now() < deadline {
        if let Some(api_key) = read_api_key(&home) {
            let client = SyncthingClient::new(
                endpoint.clone(),
                ApiKey::new(api_key).expect("generated API key should be non-empty"),
            )
            .expect("client should construct");
            if client.probe().is_ok() {
                return;
            }
        }
        thread::sleep(Duration::from_millis(250));
    }

    panic!("Syncthing did not expose a compatible REST API within 15 seconds");
}

fn isolated_home() -> std::path::PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after UNIX epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("textora-sync-real-{}-{timestamp}", std::process::id()))
}

fn reserve_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("test port should bind");
    listener.local_addr().expect("test port should be readable").port()
}

fn read_api_key(home: &std::path::Path) -> Option<String> {
    let config = fs::read_to_string(home.join("config.xml")).ok()?;
    let start = config.find("<apikey>")? + "<apikey>".len();
    let end = config[start..].find("</apikey>")? + start;
    let api_key = config[start..end].trim();
    (!api_key.is_empty()).then(|| api_key.to_owned())
}
