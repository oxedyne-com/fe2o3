//! `#[derive(ToDat, FromDat)]`: every field goes through its own `ToDat`/`FromDat`, so any field
//! type that has them (a derived type, `Option`, `Vec`, a tuple, a `BTreeMap`) nests. No `Default`.
//!
//! The encoding, which is stable:
//! - a struct with named fields is a `Dat::Map` from each field name to its value,
//! - a tuple struct is a `Dat::List` of its fields in order, a unit struct `Dat::Empty`,
//! - an enum variant is a one-entry `Dat::Map` from the variant name to its payload, which is
//!   `Dat::Empty`, a `Dat::List` or a `Dat::Map` by the same rules.
//!
//! Decoding refuses a missing field, an unknown field, a list of the wrong length and an unknown
//! variant, naming the type, the variant and the field.

use proc_macro2::TokenStream;
use quote::{
    format_ident,
    quote,
};
use syn::{
    DeriveInput,
    Fields,
};


enum Side {
    To,
    From,
}

/// Wraps an `Outcome` expression in an early return, since generated code is held to the same
/// rule as written code: no `?`.
fn tri(expr: TokenStream) -> TokenStream {
    quote! {
        match #expr {
            ::core::result::Result::Ok(v)  => v,
            ::core::result::Result::Err(e) => return ::core::result::Result::Err(e),
        }
    }
}

fn key_of(ident: &syn::Ident) -> String {
    let s = ident.to_string();
    match s.strip_prefix("r#") {
        Some(rest)  => rest.to_string(),
        None        => s,
    }
}

fn bounded(ast: &DeriveInput, side: Side) -> syn::Generics {
    let mut generics = ast.generics.clone();
    let params: Vec<syn::Ident> = generics.type_params().map(|p| p.ident.clone()).collect();
    if !params.is_empty() {
        let clause = generics.make_where_clause();
        for p in params {
            let pred: syn::WherePredicate = match side {
                Side::To    => syn::parse_quote! { #p: ::oxedyne_fe2o3_jdat::conv::ToDat },
                Side::From  => syn::parse_quote! { #p: ::oxedyne_fe2o3_jdat::conv::FromDat },
            };
            clause.predicates.push(pred);
        }
    }
    generics
}

/// The encoding of a set of fields already bound to `bindings`.
fn encode_fields(fields: &Fields, bindings: &[syn::Ident], ty: &str) -> TokenStream {
    match fields {
        Fields::Named(named) => {
            let mut puts = Vec::new();
            for (f, b) in named.named.iter().zip(bindings) {
                let key = match &f.ident {
                    Some(ident) => key_of(ident),
                    None        => String::new(),
                };
                let enc = tri(quote! { __d::enc(#b, #ty, #key) });
                puts.push(quote! { __d::put(&mut __m, #key, #enc); });
            }
            quote! {{
                let mut __m = __d::DaticleMap::new();
                #(#puts)*
                __d::Dat::Map(__m)
            }}
        },
        Fields::Unnamed(_) => {
            let mut items = Vec::new();
            for (i, b) in bindings.iter().enumerate() {
                let at = i.to_string();
                items.push(tri(quote! { __d::enc(#b, #ty, #at) }));
            }
            quote! { __d::Dat::List(::std::vec![#(#items),*]) }
        },
        Fields::Unit => quote! { __d::Dat::Empty },
    }
}

/// Decodes `__p` into the fields, then builds `ctor`.
fn decode_fields(fields: &Fields, ctor: TokenStream, ty: &str) -> TokenStream {
    match fields {
        Fields::Named(named) => {
            let mut lets = Vec::new();
            let mut inits = Vec::new();
            for (i, f) in named.named.iter().enumerate() {
                let ident = match &f.ident {
                    Some(ident) => ident.clone(),
                    None        => format_ident!("__f{}", i),
                };
                let key = key_of(&ident);
                let v = format_ident!("__v{}", i);
                let get = tri(quote! { __d::field(&mut __m, #ty, #key) });
                lets.push(quote! { let #v = #get; });
                inits.push(quote! { #ident: #v });
            }
            let take = tri(quote! { __d::take_map(__p, #ty) });
            let done = tri(quote! { __d::done(__m, #ty) });
            quote! {{
                let mut __m = #take;
                #(#lets)*
                #done;
                ::core::result::Result::Ok(#ctor { #(#inits),* })
            }}
        },
        Fields::Unnamed(unnamed) => {
            let n = unnamed.unnamed.len();
            let mut lets = Vec::new();
            let mut vs = Vec::new();
            for i in 0..n {
                let v = format_ident!("__v{}", i);
                let get = tri(quote! { __d::item(&mut __it, #ty, #i) });
                lets.push(quote! { let #v = #get; });
                vs.push(v);
            }
            let take = tri(quote! { __d::take_list(__p, #ty, #n) });
            quote! {{
                let mut __it = #take;
                #(#lets)*
                ::core::result::Result::Ok(#ctor ( #(#vs),* ))
            }}
        },
        Fields::Unit => {
            let take = tri(quote! { __d::take_unit(__p, #ty) });
            quote! {{
                #take;
                ::core::result::Result::Ok(#ctor)
            }}
        },
    }
}

/// The bindings a pattern over `fields` introduces, and the pattern itself.
fn pattern(fields: &Fields, path: TokenStream) -> (TokenStream, Vec<syn::Ident>) {
    match fields {
        Fields::Named(named) => {
            let mut binds = Vec::new();
            let mut pats = Vec::new();
            for (i, f) in named.named.iter().enumerate() {
                let b = format_ident!("__b{}", i);
                if let Some(ident) = &f.ident {
                    pats.push(quote! { #ident: #b });
                }
                binds.push(b);
            }
            (quote! { #path { #(#pats),* } }, binds)
        },
        Fields::Unnamed(unnamed) => {
            let binds: Vec<syn::Ident> =
                (0..unnamed.unnamed.len()).map(|i| format_ident!("__b{}", i)).collect();
            (quote! { #path ( #(#binds),* ) }, binds)
        },
        Fields::Unit => (path, Vec::new()),
    }
}

pub fn derive_to_dat(ast: DeriveInput) -> TokenStream {
    let name = &ast.ident;
    let tname = name.to_string();
    let generics = bounded(&ast, Side::To);
    let (impl_g, ty_g, where_c) = generics.split_for_impl();
    let body = match &ast.data {
        syn::Data::Struct(st) => {
            let (pat, binds) = pattern(&st.fields, quote! { Self });
            let enc = encode_fields(&st.fields, &binds, &tname);
            match &st.fields {
                Fields::Unit    => quote! { ::core::result::Result::Ok(#enc) },
                _               => quote! {
                    let #pat = self;
                    ::core::result::Result::Ok(#enc)
                },
            }
        },
        syn::Data::Enum(en) => {
            let mut arms = Vec::new();
            for v in &en.variants {
                let vident = &v.ident;
                let vname = key_of(vident);
                let vty = format!("{}::{}", tname, vname);
                let (pat, binds) = pattern(&v.fields, quote! { Self::#vident });
                let enc = encode_fields(&v.fields, &binds, &vty);
                arms.push(quote! { #pat => ::core::result::Result::Ok(__d::variant(#vname, #enc)), });
            }
            if arms.is_empty() {
                quote! { match *self {} }
            } else {
                quote! {
                    match self {
                        #(#arms)*
                    }
                }
            }
        },
        syn::Data::Union(_) => return syn::Error::new_spanned(
            name, "ToDat cannot be derived for a union.").to_compile_error(),
    };
    quote! {
        impl #impl_g ::oxedyne_fe2o3_jdat::conv::ToDat for #name #ty_g #where_c {
            #[allow(unused_variables, unused_imports)]
            fn to_dat(&self) -> ::oxedyne_fe2o3_jdat::derive::Outcome<::oxedyne_fe2o3_jdat::Dat> {
                use ::oxedyne_fe2o3_jdat::derive as __d;
                #body
            }
        }
    }
}

pub fn derive_from_dat(ast: DeriveInput) -> TokenStream {
    let name = &ast.ident;
    let tname = name.to_string();
    let generics = bounded(&ast, Side::From);
    let (impl_g, ty_g, where_c) = generics.split_for_impl();
    let body = match &ast.data {
        syn::Data::Struct(st) => {
            let dec = decode_fields(&st.fields, quote! { Self }, &tname);
            quote! {
                let __p = dat;
                #dec
            }
        },
        syn::Data::Enum(en) => {
            let mut arms = Vec::new();
            for v in &en.variants {
                let vident = &v.ident;
                let vname = key_of(vident);
                let vty = format!("{}::{}", tname, vname);
                let dec = decode_fields(&v.fields, quote! { Self::#vident }, &vty);
                arms.push(quote! { #vname => #dec, });
            }
            let take = tri(quote! { __d::take_variant(dat, #tname) });
            quote! {
                let (__n, __p) = #take;
                match __n.as_str() {
                    #(#arms)*
                    _ => ::core::result::Result::Err(__d::unknown_variant(#tname, &__n)),
                }
            }
        },
        syn::Data::Union(_) => return syn::Error::new_spanned(
            name, "FromDat cannot be derived for a union.").to_compile_error(),
    };
    quote! {
        impl #impl_g ::oxedyne_fe2o3_jdat::conv::FromDat for #name #ty_g #where_c {
            #[allow(unused_variables, unused_mut, unused_imports)]
            fn from_dat(dat: ::oxedyne_fe2o3_jdat::Dat)
                -> ::oxedyne_fe2o3_jdat::derive::Outcome<Self>
            {
                use ::oxedyne_fe2o3_jdat::derive as __d;
                #body
            }
        }
    }
}
