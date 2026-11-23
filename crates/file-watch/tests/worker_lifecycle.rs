use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use file_watch::WatchWorker;
use notify::RecommendedWatcher;

#[test]
fn dropping_worker_queues_shutdown_during_backend_initialization() {
    let timeout = Duration::from_secs(2);
    let (entered, wait_for_initialization) = mpsc::channel();
    let (release, wait_for_release) = mpsc::channel();
    let (worker, events) = WatchWorker::spawn(
        "test-file-watch",
        (),
        move |_| -> notify::Result<RecommendedWatcher> {
            entered.send(()).expect("initialization should report entry");
            wait_for_release.recv().expect("test should release initialization");
            Err(notify::Error::generic("injected initialization failure"))
        },
        move |watcher, commands, events| {
            assert!(watcher.is_err());
            commands.recv_timeout(timeout).expect("shutdown should be queued");
            events.send(()).expect("worker should acknowledge exit");
        },
    )
    .expect("test worker should start");
    wait_for_initialization.recv_timeout(timeout).expect("initialization should start");

    let (returned, wait_for_shutdown) = mpsc::channel();
    let caller = thread::spawn(move || {
        drop(worker);
        returned.send(()).expect("caller should report completion");
    });
    let shutdown_result = wait_for_shutdown.recv_timeout(timeout);
    release.send(()).expect("test should release initialization");
    caller.join().expect("shutdown caller should exit");

    shutdown_result.expect("shutdown should return while initialization is blocked");
    events.recv_timeout(timeout).expect("worker should receive queued shutdown after release");
}
