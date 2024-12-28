use crate::{
    lines::TextLines,
};

use oxedize_fe2o3_core::prelude::*;

use std::{
    fmt,
    marker::{
        Sync,
    },
    rc::Rc,
    sync::{
        Arc,
        RwLock,
        RwLockWriteGuard,
    },
};


#[derive(Clone, Debug)]
pub enum AccessibleText<
    T: Clone + fmt::Debug + Default + Sync,
    D: Clone + fmt::Debug + Default + Sync,
> {
    ThreadShared(Arc<RwLock<TextLines<T, D>>>),
    Shared(Rc<RwLock<TextLines<T, D>>>),
}

impl<
    T: Clone + fmt::Debug + Default + Sync,
    D: Clone + fmt::Debug + Default + Sync,
>
    Default for AccessibleText<T, D>
{
    fn default() -> Self {
        Self::Shared(Rc::new(RwLock::new(TextLines::default())))
    }
}

impl<
    T: Clone + fmt::Debug + Default + Sync,
    D: Clone + fmt::Debug + Default + Sync,
>
    AccessibleText<T, D>
{
    pub fn get_text_lines(&self) -> Outcome<RwLockWriteGuard<'_, TextLines<T, D>>> {
        match self {
            AccessibleText::ThreadShared(locked) => match locked.write() {
                Ok(guard) => Ok(guard),
                Err(_) => Err(err!(
                    "Failed to acquire write lock.";
                Poisoned, Lock)),
            }
            AccessibleText::Shared(locked) => match locked.write() {
                Ok(guard) => Ok(guard),
                Err(_) => Err(err!(
                    "Failed to acquire write lock.";
                Poisoned, Lock)),
            }
        }
    }

    pub fn get_text_lines_mut(&mut self) -> Outcome<RwLockWriteGuard<'_, TextLines<T, D>>> {
        match self {
            AccessibleText::ThreadShared(locked) => match locked.write() {
                Ok(guard) => Ok(guard),
                Err(_) => Err(err!(
                    "Failed to acquire write lock.";
                Poisoned, Lock)),
            }
            AccessibleText::Shared(locked) => match locked.write() {
                Ok(guard) => Ok(guard),
                Err(_) => Err(err!(
                    "Failed to acquire write lock.";
                Poisoned, Lock)),
            }
        }
    }
}
