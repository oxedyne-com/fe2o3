#[macro_export]
macro_rules! new_type {
    ($newtyp:ident, $wrapped:ty, $($derive:ty),* $(,)?) => {

        #[repr(transparent)]
        #[derive($($derive),*)]
        pub struct $newtyp(pub $wrapped);
        
        impl std::ops::Deref for $newtyp {
            type Target = $wrapped;
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl std::ops::DerefMut for $newtyp {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }
        
        impl oxedize_fe2o3_core::conv::IntoInner for $newtyp {
            type Inner = $wrapped;
            fn into_inner(self) -> Self::Inner {
                self.0
            }
        }
    };
    ($newtyp:ident, $wrapped:ty) => {

        #[repr(transparent)]
        pub struct $newtyp(pub $wrapped);
        
        impl std::ops::Deref for $newtyp {
            type Target = $wrapped;
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl std::ops::DerefMut for $newtyp {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }
        
        impl oxedize_fe2o3_core::conv::IntoInner for $newtyp {
            type Inner = $wrapped;
            fn into_inner(self) -> Self::Inner {
                self.0
            }
        }
    };
}

#[macro_export]
macro_rules! new_type_priv {
    ($newtyp:ident, $wrapped:ty, $($derive:ty),* $(,)?) => {

        #[repr(transparent)]
        #[derive($($derive),*)]
        pub struct $newtyp($wrapped);
        
        impl std::ops::Deref for $newtyp {
            type Target = $wrapped;
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }
        
        impl oxedize_fe2o3_core::conv::IntoInner for $newtyp {
            type Inner = $wrapped;
            fn into_inner(self) -> Self::Inner {
                self.0
            }
        }
    };
    ($newtyp:ident, $wrapped:ty) => {

        #[repr(transparent)]
        pub struct $newtyp($wrapped);
        
        impl std::ops::Deref for $newtyp {
            type Target = $wrapped;
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl oxedize_fe2o3_core::conv::IntoInner for $newtyp {
            type Inner = $wrapped;
            fn into_inner(self) -> Self::Inner {
                self.0
            }
        }
    };
}

#[macro_export]
macro_rules! new_type_gen {
    ($newtyp:ident, $($bound:ident),* ; $($derive:ty),* $(,)? ) => {

        #[derive($($derive),*)]
        pub struct $newtyp<N: $($bound)+>(pub N);
        
        impl<N: $($bound)+> std::ops::Deref for $newtyp<N> {
            type Target = N;
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }
    }
}

/// Creates iteration capabilities for enums by implementing methods to access variants as arrays
/// and count them.
///
/// # Features
/// - `variants()`: Returns a const array containing all enum variants
/// - `num_of_variants()`: Returns the total number of variants as a const usize
///
/// # Use Cases
/// - Error handling and status reporting
/// - State machine implementations
/// - Configuration options
/// - UI element types
/// - Protocol message types
/// - Testing different cases systematically
///
/// # Examples
///
/// ## Basic enum without fields
/// ```rust
/// #[derive(Clone, Copy, Debug)]
/// pub enum LogLevel {
///     Debug,
///     Info,
///     Warning,
///     Error,
/// }
///
/// new_enum!(LogLevel; Debug, Info, Warning, Error);
///
/// // Iterate through all log levels.
/// for level in LogLevel::variants() {
///     println!("Processing log level: {:?}", level);
/// }
/// ```
///
/// ## Enum with fields
/// ```rust
/// #[derive(Debug)]
/// pub enum DatabaseError {
///     ConnectionFailed(String),
///     QueryError(i32, String),
///     Timeout(u64),
/// }
///
/// new_enum!(DatabaseError; 
///     ConnectionFailed("connection refused".to_string()),
///     QueryError(-1, "syntax error".to_string()),
///     Timeout(30),
/// );
///
/// // Use in error handling.
/// for error in DatabaseError::variants() {
///     println!("Error handler for: {:?}", error);
/// }
/// ```
///
/// ## State machine states
/// ```rust
/// #[derive(Clone, Copy, Debug, PartialEq)]
/// pub enum ConnectionState {
///     Disconnected,
///     Connecting,
///     Connected,
///     Closing,
/// }
///
/// new_enum!(ConnectionState; Disconnected, Connecting, Connected, Closing);
///
/// // Validate state transitions.
/// fn is_valid_transition(from: ConnectionState, to: ConnectionState) -> bool {
///     match (from, to) {
///         (ConnectionState::Disconnected, ConnectionState::Connecting) => true,
///         (ConnectionState::Connecting, ConnectionState::Connected) => true,
///         (ConnectionState::Connected, ConnectionState::Closing) => true,
///         (ConnectionState::Closing, ConnectionState::Disconnected) => true,
///         _ => false,
///     }
/// }
///
/// // Test all possible state transitions.
/// for from_state in ConnectionState::variants() {
///     for to_state in ConnectionState::variants() {
///         println!("{:?} -> {:?}: {}", 
///             from_state, 
///             to_state, 
///             is_valid_transition(from_state, to_state)
///         );
///     }
/// }
/// ```
///
/// ## UI Components
/// ```rust
/// #[derive(Debug)]
/// pub enum DialogType {
///     Info(String),
///     Warning(String, bool),
///     Error(String, String), // message and technical details
/// }
///
/// new_enum!(DialogType;
///     Info("Information".to_string()),
///     Warning("Warning".to_string(), true),
///     Error("Error".to_string(), "Stack trace".to_string()),
/// );
///
/// // Test rendering of all dialog types.
/// fn test_dialog_rendering() {
///     for dialog in DialogType::variants() {
///         println!("Rendering dialog: {:?}", dialog);
///     }
/// }
/// ```
///
/// # Note
/// - **Variant Duplication Required**: This macro requires duplicating enum variants to avoid
///   compile-time overhead from proc-macro parsing. This trade-off is acceptable for enums 
///   with variants that aren't likely to change frequently (like Country, Color, etc.).
///   The duplication ensures fast compile times while providing convenient iteration and 
///   random selection functionality.
/// - The macro generates const methods, allowing use in const contexts
/// - Fields in variants must implement Copy or provide owned values
/// - Useful for exhaustive testing and validation
/// - Can be combined with other derive macros like Clone, Copy, Debug
#[macro_export]
macro_rules! new_enum {
    ($name:ident; $($variant:ident $(($($field:expr),+))?,)*) => {
        impl $name {
            const fn variants() -> [$name; 0 $(+ (1 $(+ <[()]>::len(&[$($field),+]))?))*] {
                [$($name::$variant $(($($field),+))?),*]
            }
            const fn num_of_variants() -> usize {
                0 $(+ (1 $(+ <[()]>::len(&[$($field),+]))?))*
            }
            pub fn rand() -> Self {
                let index = ::oxedize_fe2o3_core::rand::Rand::in_range(0, Self::num_of_variants());
                Self::variants()[index]
            }
        }
    }
}
