//! Provides a simple boolean channel for threads.  Full credit to [Denis
//! Kolodin](https://stackoverflow.com/questions/35883390/how-to-check-if-a-thread-has-finished-in-rust)
//! and his [`thread-control`](https://crates.io/crates/thread-control) crate.  I've started mainly
//! by renaming a few things.  There is no way in vanilla Rust to know when a thread has ended,
//! given that operating system threads are used.  The basic idea of this nice workaround is to share a reference
//! counted pointer to a boolean, the "semaphore" created in the parent thread, with the child
//! thread.  A non-owning reference to the semaphore, a "sentinel" is also created and can be used
//! in the parent thread to change the value of the boolean, or to detect when it has been dropped,
//! which we assume coincides with the completion of the child thread.
use crate::{
    channels::Simplex,
};

use std::{
    sync::{
        atomic::{
            AtomicBool,
            Ordering,
        },
        Arc,
        Mutex,
        Weak,
    },
    thread,
};


pub fn thread_channel() -> (Semaphore, Sentinel) {
    let flag = Semaphore::new();
    let control = flag.to_sentinel();
    (flag, control)
}

#[derive(Clone, Debug)]
pub struct ThreadController<T> {
    pub chan: Simplex<T>,
    pub hopt: Arc<Mutex<Option<thread::JoinHandle<()>>>>,
    pub sema: Semaphore,
}

impl<T> ThreadController<T> {
    pub fn new(
        chan: Simplex<T>,
        hopt: Arc<Mutex<Option<thread::JoinHandle<()>>>>,
        sema: Semaphore,
    )
        -> Self
    {
        Self {
            chan,
            hopt,
            sema,
        }
    }
}

/// A semaphore contains two flags, one indicating whether the thread is, or should be, alive.  The
/// other indicates whether the thread should be interrupted.  Pass the semaphore to a child thread
/// so that its Drop function is called when the thread ends.
#[derive(Clone, Debug, Default)]
pub struct Semaphore {
    alive: Arc<AtomicBool>,
    interrupt: Arc<AtomicBool>,
}

impl Drop for Semaphore {
    fn drop(&mut self) {
        if thread::panicking() {
            (*self.interrupt).store(true, Ordering::Relaxed)
        }
    }
}

impl Semaphore {

    /// Creates new flag.
    pub fn new() -> Self {
        Self {
            alive: Arc::new(AtomicBool::new(true)),
            interrupt: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Bring the semaphore into scope so that the compiler adds a Drop function at the end of the
    /// scope (i.e. the thread).  This is used by `oxedyne_fe2o3_log::logger::Logger` because hiding the
    /// semaphore inside the `LOG` singleton would not yield a call to its `Drop` function, since
    /// `LOG` is static.  Normally you would activate a semaphore inside a child thread with a
    /// `while semaphore.alive() {}` loop.
    pub fn touch(&self) {}

    /// Yields a new `Sentinel` which can be used to monitor and control this semaphore from code
    /// executed within the child thread.
    pub fn to_sentinel(&self) -> Sentinel {
        Sentinel {
            alive: Arc::downgrade(&self.alive),
            interrupt: self.interrupt.clone(),
        }
    }

    /// Return the status of the semaphore alive flag.
    ///
    /// # Panics
    ///
    /// This method panics if the interrupt flag has been set.
    pub fn alive_or_panic(&self) -> bool {
        if (*self.interrupt).load(Ordering::Relaxed) {
            panic!("thread interrupted by thread-contol");
        }
        (*self.alive).load(Ordering::Relaxed)
    }

    /// Check the semaphore alive flag without any possibility of panicking.
    pub fn is_alive(&self) -> bool {
        (*self.alive).load(Ordering::Relaxed) && !(*self.interrupt).load(Ordering::Relaxed)
    }

    /// Consume the `Semaphore` and set its interrupt flag to true, which causes a thread panic
    /// next time the `alive_or_panic` method is called.
    pub fn interrupt(self) {
        (self.interrupt).store(true, Ordering::Relaxed)
    }
}

/// `Sentinel` is used to monitor and change the state of the `Semaphore`.
#[derive(Clone, Debug, Default)]
pub struct Sentinel {
    alive: Weak<AtomicBool>,
    interrupt: Arc<AtomicBool>,
}

impl Sentinel {

    /// Set the interrupt state of the associated `Semaphore` to true, which causes a thread panic
    /// next time the sempahore `alive_or_panic` method is called.
    pub fn interrupt(&self) {
        (*self.interrupt).store(true, Ordering::Relaxed)
    }

    /// Set the alive state of the associated `Semaphore` to false.
    pub fn stop(&self) {
        self.alive.upgrade().map(|flag| {
            (*flag).store(false, Ordering::Relaxed)
        });
    }

    /// Returns `true` if the associated `Semaphore` alive state has been set to `false`.
    pub fn is_finished(&self) -> bool {
        self.alive.upgrade().is_none()
    }

    /// Return `true` if the associated `Sempahore` was interrupted or panicked for some other
    /// reason.
    pub fn was_interrupted(&self) -> bool {
        (*self.interrupt).load(Ordering::Relaxed)
    }
}
