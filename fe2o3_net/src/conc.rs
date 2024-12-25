use std::{
    future::Future,
    pin::Pin,
};


pub trait AsyncReadIterator {
    type Item: Send;

    fn next<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = Option<Self::Item>> + Send + 'a>>;
}
