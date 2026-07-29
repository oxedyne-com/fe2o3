//! One convenience macro, for the shape of failure this crate meets most.

/// Unwraps an `Option`, returning a tagged error when it is absent.
///
/// `fe2o3_core` gives `ok!` for the `Result` case and nothing for the `Option`
/// case, so a loader that asks a model for a weight it does not carry would
/// otherwise be written as a five-line `match` two dozen times over.
#[macro_export]
macro_rules! some {
	($opt:expr, $($arg:expr),+ $(,)?) => {
		match $opt {
			Some(v) => v,
			None => return Err(err!($($arg),+ ; Invalid, Missing)),
		}
	};
}
