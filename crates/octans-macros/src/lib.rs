//! `octans-macros` — the `#[node]` authoring attribute.
//!
//! Write a node as a plain struct (its fields = construction-time config) plus an `impl` block
//! whose `process` method has a *typed* signature. Parameters are classified by attribute:
//!
//! ```ignore
//! #[node(id = "octans.std.threshold", out = "mask")]
//! impl Threshold {
//!     fn process(
//!         &self,
//!         #[ctx] ctx: &Context,                 // shared read-mostly globals (optional)
//!         #[local] s: &mut ThrState,            // per-instance private state (optional; State: Default)
//!         #[param(default = 128u8)] thr: &u8,   // an input port with a default (a parameter)
//!         image: &Image,                        // a plain input port
//!     ) -> Image { /* returns the `out` port */ }
//! }
//! ```
//!
//! The macro derives the whole `Node` impl: `inputs()`/`outputs()` from the typed signature
//! (via `RegisteredType`), `new_local()` from the `#[local]` state's `Default`, and the
//! type-erase glue (`inputs.get::<T>` / `local.downcast_mut::<S>` / `outputs.set`).
//!
//! v0 scope: inputs are by-reference (`&T`); `#[local]` is `&mut S` (`S: Default`); a single
//! output named by `out` (default `"out"`), or none for a `()` return. Multiple outputs and
//! per-lane replication land later.

use proc_macro::TokenStream;
use quote::quote;
use syn::{
    parse_macro_input, punctuated::Punctuated, Expr, FnArg, ImplItem, ItemImpl, Lit, MetaNameValue,
    Pat, ReturnType, Token, Type,
};

enum Kind {
    Ctx,
    Local(Type),
    Input { elem: Type, default: Option<Expr> },
}

struct Param {
    name: syn::Ident,
    kind: Kind,
}

fn elem_of(ty: &Type) -> Type {
    match ty {
        Type::Reference(r) => (*r.elem).clone(),
        other => other.clone(),
    }
}

#[proc_macro_attribute]
pub fn node(attr: TokenStream, item: TokenStream) -> TokenStream {
    // ---- attribute args: id = "...", out = "..." ----
    let args =
        parse_macro_input!(attr with Punctuated::<MetaNameValue, Token![,]>::parse_terminated);
    let mut id: Option<String> = None;
    let mut out_name = String::from("out");
    for nv in args {
        let key = nv
            .path
            .get_ident()
            .map(|i| i.to_string())
            .unwrap_or_default();
        let val = match &nv.value {
            Expr::Lit(e) => match &e.lit {
                Lit::Str(s) => s.value(),
                _ => String::new(),
            },
            _ => String::new(),
        };
        match key.as_str() {
            "id" => id = Some(val),
            "out" => out_name = val,
            other => {
                return syn::Error::new_spanned(
                    &nv.path,
                    format!("#[node]: unknown argument `{other}` (expected `id` or `out`)"),
                )
                .to_compile_error()
                .into()
            }
        }
    }
    let id = match id {
        Some(s) => s,
        None => {
            return syn::Error::new(
                proc_macro2::Span::call_site(),
                "#[node] requires `id = \"...\"`",
            )
            .to_compile_error()
            .into()
        }
    };

    // ---- find/rename `process`, strip the param-classifying attrs from the re-emitted copy ----
    let mut impl_block = parse_macro_input!(item as ItemImpl);
    let mut sig = None;
    for it in &mut impl_block.items {
        if let ImplItem::Fn(f) = it {
            if f.sig.ident == "process" {
                sig = Some(f.sig.clone());
                f.sig.ident = syn::Ident::new("__node_run", f.sig.ident.span());
                for arg in f.sig.inputs.iter_mut() {
                    if let FnArg::Typed(pt) = arg {
                        pt.attrs.retain(|a| {
                            !(a.path().is_ident("param")
                                || a.path().is_ident("ctx")
                                || a.path().is_ident("local"))
                        });
                    }
                }
            }
        }
    }
    let sig = match sig {
        Some(s) => s,
        None => {
            return syn::Error::new_spanned(
                &impl_block,
                "#[node] impl must contain `fn process(&self, ...)`",
            )
            .to_compile_error()
            .into()
        }
    };
    let self_ty = (*impl_block.self_ty).clone();

    // ---- classify parameters (in source order) ----
    let mut params: Vec<Param> = Vec::new();
    for arg in sig.inputs.iter() {
        let FnArg::Typed(pt) = arg else { continue }; // skip &self
        let name = match &*pt.pat {
            Pat::Ident(pi) => pi.ident.clone(),
            other => {
                return syn::Error::new_spanned(other, "#[node] params must be plain identifiers")
                    .to_compile_error()
                    .into()
            }
        };
        let mut kind: Option<Kind> = None;
        for a in &pt.attrs {
            if a.path().is_ident("ctx") {
                kind = Some(Kind::Ctx);
            } else if a.path().is_ident("local") {
                kind = Some(Kind::Local(elem_of(&pt.ty)));
            } else if a.path().is_ident("param") {
                let default = match a.parse_args::<MetaNameValue>() {
                    Ok(nv) if nv.path.is_ident("default") => Some(nv.value),
                    _ => {
                        return syn::Error::new_spanned(a, "#[param] expects `default = <expr>`")
                            .to_compile_error()
                            .into()
                    }
                };
                kind = Some(Kind::Input {
                    elem: elem_of(&pt.ty),
                    default,
                });
            }
        }
        let kind = kind.unwrap_or_else(|| Kind::Input {
            elem: elem_of(&pt.ty),
            default: None,
        });
        params.push(Param { name, kind });
    }

    let out_ty: Option<Type> = match &sig.output {
        ReturnType::Default => None,
        ReturnType::Type(_, t) => Some((**t).clone()),
    };

    // ---- generate fragments ----
    let in_ports = params.iter().filter_map(|p| match &p.kind {
        Kind::Input { elem, default } => {
            let n = p.name.to_string();
            Some(match default {
                Some(e) => quote! {
                    ::octans_core::PortSpec::with_default(
                        #n,
                        <#elem as ::octans_core::RegisteredType>::type_spec(),
                        ::octans_core::Value::new::<#elem>(#e),
                    )
                },
                None => quote! {
                    ::octans_core::PortSpec::new(#n, <#elem as ::octans_core::RegisteredType>::type_spec())
                },
            })
        }
        _ => None,
    });

    let outputs_method = match &out_ty {
        Some(t) => quote! {
            vec![ ::octans_core::PortSpec::new(#out_name, <#t as ::octans_core::RegisteredType>::type_spec()) ]
        },
        None => quote! { ::std::vec::Vec::new() },
    };

    let new_local_body = params
        .iter()
        .find_map(|p| match &p.kind {
            Kind::Local(s) => {
                Some(quote! { ::std::boxed::Box::new(<#s as ::core::default::Default>::default()) })
            }
            _ => None,
        })
        .unwrap_or_else(|| quote! { ::std::boxed::Box::new(()) });

    let binds = params.iter().map(|p| {
        let name = &p.name;
        match &p.kind {
            Kind::Ctx => quote! { let #name = _ctx; },
            Kind::Local(s) => quote! {
                let #name: &mut #s = _local
                    .downcast_mut::<#s>()
                    .expect("octans: local state type mismatch");
            },
            Kind::Input { elem, .. } => {
                let ns = name.to_string();
                quote! { let #name = _inputs.get::<#elem>(#ns); }
            }
        }
    });

    let call_args = params.iter().map(|p| &p.name);
    let run = quote! { self.__node_run( #( #call_args ),* ) };
    let body_tail = match &out_ty {
        Some(_) => quote! { let __ret = #run; _outputs.set(#out_name, __ret); },
        None => quote! { let () = #run; },
    };

    quote! {
        #impl_block

        impl ::octans_core::Node for #self_ty {
            fn type_id(&self) -> &'static str { #id }
            fn inputs(&self) -> ::std::vec::Vec<::octans_core::PortSpec> {
                vec![ #( #in_ports ),* ]
            }
            fn outputs(&self) -> ::std::vec::Vec<::octans_core::PortSpec> {
                #outputs_method
            }
            fn new_local(&self) -> ::std::boxed::Box<dyn ::std::any::Any + ::std::marker::Send> {
                #new_local_body
            }
            fn process(
                &self,
                _ctx: &::octans_core::Context,
                _local: &mut dyn ::std::any::Any,
                _inputs: &::octans_core::Inputs,
                _outputs: &mut ::octans_core::Outputs,
            ) {
                #( #binds )*
                #body_tail
            }
        }
    }
    .into()
}
