//! Background ownership of platform file watchers.

use std::io;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use notify::{Config, EventHandler, RecommendedWatcher, Watcher};

pub fn recommended_watcher(handler: impl EventHandler) -> notify::Result<RecommendedWatcher> {
    RecommendedWatcher::new(handler, Config::default())
}

pub struct WatchWorker<Command> {
    commands: mpsc::Sender<Command>,
    running: Option<(Command, JoinHandle<()>)>,
}

impl<Command> WatchWorker<Command> {
    /// Starts the backend on the worker thread. The event loop receives the backend outcome.
    pub fn spawn<W: Watcher, Output: Send + 'static>(
        name: &str,
        shutdown: Command,
        create_watcher: impl FnOnce(&mpsc::Sender<Command>) -> notify::Result<W> + Send + 'static,
        run: impl FnOnce(notify::Result<W>, mpsc::Receiver<Command>, mpsc::Sender<Output>)
        + Send
        + 'static,
    ) -> io::Result<(Self, mpsc::Receiver<Output>)>
    where
        Command: Send + 'static,
    {
        let (commands, command_receiver) = mpsc::channel();
        let (output_sender, output_receiver) = mpsc::channel();
        let initialization_commands = commands.clone();
        let worker = thread::Builder::new().name(name.to_owned()).spawn(move || {
            let watcher = create_watcher(&initialization_commands);
            drop(initialization_commands);
            run(watcher, command_receiver, output_sender);
        })?;
        Ok((Self { commands, running: Some((shutdown, worker)) }, output_receiver))
    }

    pub fn send(&self, command: Command) -> Result<(), mpsc::SendError<Command>> {
        self.commands.send(command)
    }

    pub fn shutdown(&mut self) {
        let Some((shutdown, worker)) = self.running.take() else { return };
        let _ = self.commands.send(shutdown);
        // A watcher can block in platform registration or destruction. Its thread
        // owns the backend and exits once those calls return.
        if worker.is_finished() {
            let _ = worker.join();
        }
    }
}

impl<Command> Drop for WatchWorker<Command> {
    fn drop(&mut self) {
        self.shutdown();
    }
}
