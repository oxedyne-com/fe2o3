/// Use `std::convert::From` for conversion with no intermediate processing.  This trait is for
/// conversions regarded as giving the "best" outcome.  The default fallback is `From`.
pub trait BestFrom<T>: Sized + std::convert::From<T> {
    fn best_from(value: T) -> Self {
        Self::from(value)
    }
}

pub trait IntoInner {
    type Inner;
    fn into_inner(self) -> Self::Inner;
}
