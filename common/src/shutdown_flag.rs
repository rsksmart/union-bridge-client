use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::flag;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

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

    /// Ideally, this method should be used only for testing purposes
    #[cfg(feature = "generate-mocks")]
    pub fn set(&self, value: bool) {
        self.flag.store(value, Ordering::SeqCst);
    }
}
