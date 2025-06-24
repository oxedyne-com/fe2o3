use oxedyne_fe2o3_core::thread::Sentinel;

#[derive(Debug, Default)]
pub struct Handle<T> {
    id:         String,
    pub thread: Option<std::thread::JoinHandle<T>>,
    sentinel:   Sentinel,
}

impl<T> Handle<T> {

    pub fn new(
        id:         String,
        hand:       std::thread::JoinHandle<T>,
        sentinel:   Sentinel,
    ) -> Self {
        Self {
            id:         id,
            thread:     Some(hand),
            sentinel:   sentinel,
        }
    }

    pub fn id(&self) -> &String         { &self.id }
    pub fn sentinel(&self) -> &Sentinel { &self.sentinel }
}
