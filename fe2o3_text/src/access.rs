use crate::{
    lines::TextLines,
};

use oxedyne_fe2o3_core::prelude::*;

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
    /// Acquires a write guard over the underlying text lines, propagating a
    /// poisoned-lock error via the `lock_write!` macro.
    pub fn get_text_lines(&self) -> Outcome<RwLockWriteGuard<'_, TextLines<T, D>>> {
        match self {
            AccessibleText::ThreadShared(locked) => Ok(lock_write!(locked)),
            AccessibleText::Shared(locked)       => Ok(lock_write!(locked)),
        }
    }

    /// Mutable-receiver alias for [`Self::get_text_lines`]; both return an
    /// exclusive write guard, so this simply delegates.
    pub fn get_text_lines_mut(&mut self) -> Outcome<RwLockWriteGuard<'_, TextLines<T, D>>> {
        self.get_text_lines()
    }
}
