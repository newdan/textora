use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread::{self, JoinHandle};

use textora_sync::{
    ApiKey, DeviceId, EventCursor, FolderId, FolderPhase, LoopbackEndpoint, SyncEvent,
    SyncEventKind, SyncthingClient,
};

const FIXTURE: &str = include_str!("fixtures/v2_1_1_read_api.json");

struct MockServer {
    endpoint: LoopbackEndpoint,
    thread: JoinHandle<Vec<String>>,
}

impl MockServer {
    fn start(responses: Vec<String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("mock server should bind");
        let address = listener.local_addr().expect("mock server should expose address");
        let thread = thread::spawn(move || {
            let mut requests = Vec::new();
            for body in responses {
                let (mut stream, _) = listener.accept().expect("mock server should accept");
                requests.push(read_request(&mut stream));
                write_response(&mut stream, &body);
            }
            requests
        });
        let endpoint_url = format!("http://{address}");
        let endpoint = LoopbackEndpoint::parse(&endpoint_url).expect("endpoint should parse");
        Self { endpoint, thread }
    }

    fn join(self) -> Vec<String> {
        self.thread.join().expect("mock server should stop")
    }
}

fn read_request(stream: &mut TcpStream) -> String {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        let count = stream.read(&mut buffer).expect("mock server should read");
        if count == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..count]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8(request).expect("mock request should be UTF-8")
}

fn write_response(stream: &mut TcpStream, body: &str) {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).expect("mock server should write");
}

#[test]
fn v2_1_1_read_contract_maps_stable_public_projections() {
    let fixture: serde_json::Value =
        serde_json::from_str(FIXTURE).expect("fixture should contain valid JSON");
    let body = |name: &str| fixture[name].to_string();
    let server = MockServer::start(vec![
        body("system_version"),
        body("system_status"),
        body("connections"),
        body("pending_devices"),
        body("pending_folders"),
        body("folder_status"),
        body("folder_errors"),
        body("events"),
    ]);
    let client = SyncthingClient::new(
        server.endpoint.clone(),
        ApiKey::new("contract-test-key".to_owned()).expect("test API key should parse"),
    )
    .expect("client should construct");

    let instance = client.probe().expect("probe should parse fixture");
    assert_eq!(instance.version.to_string(), "2.1.1");
    assert_eq!(
        instance.device_id.as_str(),
        "ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG"
    );

    let connections = client.connections().expect("connections should parse");
    assert_eq!(connections.len(), 1);
    assert_eq!(
        connections[0].as_str(),
        "HIJKLMN-HIJKLMN-HIJKLMN-HIJKLMN-HIJKLMN-HIJKLMN-HIJKLMN-HIJKLMN"
    );

    let pending_devices = client.pending_devices().expect("pending devices should parse");
    assert_eq!(pending_devices[0].name.as_deref(), Some("Remote"));

    let pending_folders = client.pending_folders().expect("pending folders should parse");
    assert_eq!(pending_folders[0].folder_id.as_str(), "notes");
    assert_eq!(pending_folders[0].label.as_deref(), Some("Notes"));

    let folder = FolderId::new("notes".to_owned()).expect("folder ID should parse");
    let status = client.folder_status(&folder).expect("folder status should parse");
    assert!(matches!(status.phase, FolderPhase::Idle));
    assert_eq!(status.completion_percent, 100.0);
    assert!(client.folder_errors(&folder).expect("folder errors should parse").is_empty());

    assert_eq!(
        client.events_since(EventCursor(0), 1).expect("events should parse"),
        vec![SyncEvent::Remote { id: 1, kind: SyncEventKind::FolderStateChanged }]
    );

    let requests = server.join();
    assert!(requests[0].starts_with("GET /rest/system/version"));
    assert!(requests[1].starts_with("GET /rest/system/status"));
    assert!(requests[2].starts_with("GET /rest/system/connections"));
    assert!(requests[3].starts_with("GET /rest/cluster/pending/devices"));
    assert!(requests[4].starts_with("GET /rest/cluster/pending/folders"));
    assert!(requests[5].starts_with("GET /rest/db/status?folder=notes"));
    assert!(requests[6].starts_with("GET /rest/folder/errors?folder=notes"));
    assert!(requests[7].starts_with("GET /rest/events?since=0&timeout=1"));
    assert!(requests.iter().all(|request| request.contains("x-api-key: contract-test-key")));

    let _ = DeviceId::parse(
        "HIJKLMN-HIJKLMN-HIJKLMN-HIJKLMN-HIJKLMN-HIJKLMN-HIJKLMN-HIJKLMN".to_owned(),
    )
    .expect("fixture device ID should be valid");
}
