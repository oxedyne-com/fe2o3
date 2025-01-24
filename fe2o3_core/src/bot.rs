//! Provides a simple foundation for code delegated to child threads.  A `Bot` is intended to be
//! the main loop function for a child thread, `BaseMsg` and the `CtrlMsg` trait provide some
//! baseline message types that can be elaborated in your application via composition (see
//! `oxedize_fe2o3_o3db_sync::bot`).

use crate::{
    prelude::*,
    error,
    GenTag,
};

pub trait CtrlMsg {
    fn finish() -> Self;
    fn ready() -> Self;
}

pub trait ErrorMsg<T: GenTag> where error::Error<T>: std::error::Error {
    fn error(e: Error<T>) -> Self;
}

#[derive(Clone, Debug)]
pub enum BaseMsg<T: GenTag> where error::Error<T>: std::error::Error {
    Error(Error<T>),
    Finish,
    Ready,
}

impl<T: GenTag> CtrlMsg for BaseMsg<T> where error::Error<T>: std::error::Error {
    fn finish() -> Self { BaseMsg::Finish }
    fn ready() -> Self  { BaseMsg::Ready }
}

impl<T: GenTag> ErrorMsg<T> for BaseMsg<T> where error::Error<T>: std::error::Error {
    fn error(e: Error<T>) -> Self { BaseMsg::Error(e) }
}
