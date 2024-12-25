/// Credit: https://stackoverflow.com/users/4498831/boiethios 
/// https://stackoverflow.com/questions/57685567/how-to-move-values-out-of-a-vector-when-the-vector-is-immediately-discarded
pub trait Extract: Default {
    fn extract(&mut self) -> Self;
}

impl<T: Default> Extract for T {
    fn extract(&mut self) -> Self {
        std::mem::replace(self, T::default())
    }
}
