#![recursion_limit = "128"]
/// Procedural macros to derive implementations for [`FromDatMap`] and [`ToDatMap`],
/// allowing a struct to be converted from and to a [`Dat`].
///
/// Credit: https://github.com/ex0dus-0x/structmap
///

use std::collections::BTreeMap;

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{
    quote,
};
use syn::{
    self,
    DeriveInput,
};

#[proc_macro_derive(FromDatMap, attributes(skip, optional, rename))]
pub fn from_datmap(input: TokenStream) -> TokenStream {
    let ast = syn::parse_macro_input!(input as DeriveInput);

    let fields = match ast.data {
        syn::Data::Struct(st) => st.fields,
        _ => panic!("from_datmap: Implementation must be a struct."),
    };

    let skip_map = parse_valueless_attrs(&fields, "skip");
    let optional_map = parse_valueless_attrs(&fields, "optional");
    let rename_map = parse_rename_attrs(&fields);

    //let idents: Vec<&syn::Ident> = fields
    //    .iter()
    //    .filter_map(|field| field.ident.as_ref())
    //    .collect::<Vec<&syn::Ident>>();

    let mut keys: Vec<String> = Vec::new();
    let mut idents: Vec<syn::Ident> = Vec::new();
    let mut typecalls: Vec<syn::Ident> = Vec::new();
    let mut raise_error_if_missing: Vec<bool> = Vec::new();
    for field in fields {
        if let Some(ident) = field.ident {
            let name = ident.to_string();
            if !skip_map.contains_key(&name) {
                match rename_map.get(&name) {
                    Some(new_name) => keys.push(new_name.clone()),
                    None => keys.push(name.clone()),
                }
                raise_error_if_missing.push(match optional_map.get(&name) {
                    Some(()) => false,
                    None => true,
                });
                idents.push(ident.clone());
                match field.ty {
                    syn::Type::Path(typepath) => {
                        // get the type of the specified field, lowercase
                        let type_name: String = quote! {#typepath}.to_string().to_lowercase();
                        let getter_name: String = match &*type_name {
                            "u8" | "u16" | "u32" | "u64" |
                            "i8" | "i16" | "i32" | "i64" |
                            "bool" | "float32" | "float64" |
                            "bigint" | "bigdecimal" |
                            "string" | "dat" => format!("get_{}", type_name),
                            "vec < u8 >" => String::from("get_bytes"),
                            "box < dat >" => String::from("get_box"),
                            "box < option < dat > >" => String::from("get_opt_dat"),
                            "vec < dat >" => String::from("get_list"),
                            "vec < string >" => String::from("get_string_list"),
                            "btreemap < dat, dat >" | "daticlemap" => String::from("get_map"),
                            "btreemap < b32, string >" => String::from("get_b32_string_map"),
                            _ => unimplemented!(
                                "from_datmap: Cannot find an equivalent Dat for type '{}'.",
                                type_name,
                            ),
                        };

                        // initialize new Ident for codegen
                        typecalls.push(syn::Ident::new(&getter_name, Span::mixed_site()))
                    }
                    _ => unimplemented!(),
                }
            }
        }
    }

    let name = &ast.ident;
    let (impl_generics, ty_generics, where_clause) = ast.generics.split_for_impl();

    let tokens = quote! {

        impl #impl_generics FromDatMap for #name #ty_generics #where_clause {

            fn from_datmap(
                mut map: ::std::collections::BTreeMap<Dat, Dat>,
            ) -> Outcome<#name> {
                let mut st = #name::default();

                #(
                    match map.entry(Dat::Str(String::from(#keys))) {
                        ::std::collections::btree_map::Entry::Occupied(entry) => {
                            // parse out primitive value from generic type using typed call
                            let value = match entry.get().#typecalls() {
                                Some(val) => val,
                                None => return Err(err!(
                                    "from_datmap: The key for the Dat::Map pair {:?} matches \
                                    the required struct field '{}' but the value is not a \
                                    recognised type.",
                                    entry, #keys;
                                Invalid, Input)),
                            };
                            st.#idents = value;
                        },
                        _ => if #raise_error_if_missing {
                                return Err(err!(
                                "from_datmap: The required struct field '{}' cannot be found \
                                in the given Dat::Map {:?}.", #keys, map;
                                Invalid, Input));
                        },
                    }
                )*

                Ok(st)
            }
        }
    };

    tokens.into()
}

#[proc_macro_derive(ToDatMap, attributes(rename))]
pub fn to_datmap(input: TokenStream) -> TokenStream {
    let ast = syn::parse_macro_input!(input as DeriveInput);

    let fields = match ast.data {
        syn::Data::Struct(st) => st.fields,
        _ => panic!("to_datmap: Implementation must be a struct."),
    };

    let rename_map = parse_rename_attrs(&fields);

    let idents: Vec<&syn::Ident> = fields
        .iter()
        .filter_map(|field| field.ident.as_ref())
        .collect::<Vec<&syn::Ident>>();

    // convert all the field names into strings
    let keys: Vec<String> = idents
        .clone()
        .iter()
        .map(|ident| ident.to_string())
        .map(|name| match rename_map.contains_key(&name) {
            true => rename_map.get(&name).unwrap().clone(),
            false => name,
        })
        .collect::<Vec<String>>();

    let name = &ast.ident;
    let (impl_generics, ty_generics, where_clause) = ast.generics.split_for_impl();

    let tokens = quote! {

        impl #impl_generics ToDatMap for #name #ty_generics #where_clause {

            fn to_datmap(input_struct: #name) -> Dat {
                let mut map = BTreeMap::new();
                #(
                    map.insert(
                        Dat::Str(#keys.to_string()),
                        Dat::from(input_struct.#idents),
                    );
                )*
                Dat::Map(map)
            }
        }
    };

    tokens.into()
}

/// Helper method used to parse out any `rename` attribute definitions in a struct
/// marked with the ToMap trait, returning a mapping between the original field name
/// and the one being changed for later use when doing codegen.
fn parse_rename_attrs(fields: &syn::Fields) -> BTreeMap<String, String> {
    let mut rename: BTreeMap<String, String> = BTreeMap::new();
    match fields {
        syn::Fields::Named(_) => {
            // iterate over fields available and attributes
            for field in fields.iter() {
                for attr in field.attrs.iter() {
                    // parse original struct field name
                    let field_name = field.ident.as_ref().unwrap().to_string();
                    if attr.path.get_ident().unwrap().to_string() == "rename" {
                        if rename.contains_key(&field_name) {
                            panic!("parse_rename_attrs: Cannot redefine field name multiple \
                                times.");
                        }

                        // parse out name value pairs in attributes
                        // first get `lst` in #[rename(lst)]
                        match attr.parse_meta() {
                            Ok(syn::Meta::List(lst)) => {
                                // then parse key-value name
                                match lst.nested.first() {
                                    Some(syn::NestedMeta::Meta(syn::Meta::NameValue(nm))) => {
                                        // check path to be = `name`
                                        let path = nm.path.get_ident().unwrap().to_string();
                                        if path != "name" {
                                            panic!("parse_rename_attrs: Must be \
                                                `#[rename(name = 'VALUE')]`.");
                                        }

                                        let lit = match &nm.lit {
                                            syn::Lit::Str(val) => val.value(),
                                            _ => {
                                                panic!("parse_rename_attrs: Must be \
                                                    `#[rename(name = 'VALUE')]`.");
                                            }
                                        };
                                        rename.insert(field_name, lit);
                                    }
                                    _ => {
                                        panic!("parse_rename_attrs: Must be \
                                            `#[rename(name = 'VALUE')]`.");
                                    }
                                }
                            }
                            _ => {
                                panic!("parse_rename_attrs: Must be \
                                    `#[rename(name = 'VALUE')]`.");
                            }
                        }
                    }
                }
            }
        }
        _ => {
            panic!("parse_rename_attrs: Must have named fields.");
        }
    }
    rename
}

fn parse_valueless_attrs(fields: &syn::Fields, attr_name: &str) -> BTreeMap<String, ()> {
    let mut map: BTreeMap<String, ()> = BTreeMap::new();
    match fields {
        syn::Fields::Named(_) => {
            // iterate over fields available and attributes
            for field in fields.iter() {
                for attr in field.attrs.iter() {
                    // parse original struct field name
                    let field_name = field.ident.as_ref().unwrap().to_string();
                    if attr.path.get_ident().unwrap().to_string() == attr_name {
                        map.insert(field_name, ());
                    }
                }
            }
        }
        _ => {
            panic!("parse_valueless_attrs: Must have named fields.");
        }
    }
    map
}
