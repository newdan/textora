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
        let (output_sender, output_receiver) = mpsc::channel();
        let worker = Self::spawn_loop(name, shutdown, create_watcher, move |watcher, commands| {
            run(watcher, commands, output_sender);
        })?;
        Ok((worker, output_receiver))
    }

    /// Starts a watcher whose event loop delivers results through its own event source.
    pub fn spawn_loop<W: Watcher>(
        name: &str,
        shutdown: Command,
        create_watcher: impl FnOnce(&mpsc::Sender<Command>) -> notify::Result<W> + Send + 'static,
        run: impl FnOnce(notify::Result<W>, mpsc::Receiver<Command>) + Send + 'static,
    ) -> io::Result<Self>
    where
        Command: Send + 'static,
    {
        let (commands, command_receiver) = mpsc::channel();
        let initialization_commands = commands.clone();
        let worker = thread::Builder::new().name(name.to_owned()).spawn(move || {
            let watcher = create_watcher(&initialization_commands);
            drop(initialization_commands);
            run(watcher, command_receiver);
        })?;
        Ok(Self { commands, running: Some((shutdown, worker)) })
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
