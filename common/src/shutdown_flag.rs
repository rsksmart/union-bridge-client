use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::flag;

#[derive(Clone)]
pub struct ShutdownFlag {
    flag: Arc<AtomicBool>,
}

impl ShutdownFlag {
    pub fn init() -> Self {
        let shutdown_flag = ShutdownFlag {
            flag: Arc::new(AtomicBool::new(false)),
        };

        flag::register(SIGINT, Arc::clone(&shutdown_flag.flag))
            .expect("Failed to set SIGINT handler");
        flag::register(SIGTERM, Arc::clone(&shutdown_flag.flag))
            .expect("Failed to set SIGTERM handler");

        shutdown_flag
    }

    pub fn is_on(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    pub fn spawn_shutdown_handler<F>(self, shutdown_handler: F)
    where
        F: FnOnce() + Send + 'static,
    {
        thread::spawn(move || {
            while !self.is_on() {
                thread::sleep(Duration::from_secs(1));
            }
            shutdown_handler();
        });
    }

    /// To be used only in async environments
    pub async fn wait_for(self) {
        while !self.is_on() {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    pub fn set(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }
}
