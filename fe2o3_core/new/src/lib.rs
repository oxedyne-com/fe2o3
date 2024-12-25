//! Derive macro for generating a `new` constructor function.
//! 
//! This crate provides a derive macro that automatically implements a `new` constructor
//! for structs. The generated constructor takes all fields as parameters in the order
//! they are declared and creates a new instance of the struct.
//! 
//! # Example
//! 
//! ```rust
//! use oxedize_fe2o3_core_new::New;
//! 
//! #[derive(New)]
//! struct Person {
//!     name: String,
//!     age: u32,
//! }
//! 
//! // The macro generates:
//! // impl Person {
//! //     pub fn new(name: String, age: u32) -> Self {
//! //         Self { name, age }
//! //     }
//! // }
//! 
//! let person = Person::new(String::from("Alice"), 30);
//! ```
//! 
//! # Limitations
//! 
//! - Only works on structs with named fields
//! - Does not support default values for fields
//! - Does not support field-level attributes
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput, Fields};


#[proc_macro_derive(New)]
pub fn derive_new(input: TokenStream) -> TokenStream {
    // Parse the input tokens into a syntax tree
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    
    // Extract the fields from the struct
    let fields = match &input.data {
        syn::Data::Struct(data) => {
            match &data.fields {
                Fields::Named(fields) => &fields.named,
                _ => panic!("Only named fields are supported")
            }
        },
        _ => panic!("Only structs are supported")
    };
    
    // Generate the parameter list and field initialization
    let params = fields.iter().map(|f| {
        let name = &f.ident;
        let ty = &f.ty;
        quote! { #name: #ty }
    });
    
    let field_inits = fields.iter().map(|f| {
        let name = &f.ident;
        quote! { #name }
    });
    
    // Generate the implementation
    let expanded = quote! {
        impl #name {
            pub fn new(#(#params),*) -> Self {
                Self {
                    #(#field_inits),*
                }
            }
        }
    };
    
    TokenStream::from(expanded)
}
