//! `octans-macros` — the `#[node]` authoring attribute.
//!
//! Write a node as a plain struct (its fields are construction-time config) plus an `impl`
//! block whose `process` method has a *typed* signature:
//!
//! ```ignore
//! #[node(id = "octans.std.threshold", out = "mask")]
//! impl Threshold {
//!     fn process(&self, image: &Image) -> Image { /* self.thr is in scope */ }
//! }
//! ```
//!
//! The macro derives the `Node` impl: each `process` parameter becomes a named input port
//! (its type's `TypeSpec` via `RegisteredType`), the return becomes the (named) output port,
//! and the type-erase glue — `inputs.get::<T>(name)` / `outputs.set(name, ret)` — is generated.
//! This is the boilerplate the v0 slice wrote by hand, now eliminated.
//!
//! v0 scope: inputs are by-reference (`&T`); a single output named by `out` (default `"out"`),
//! or none for a `()` return. Multiple outputs, params-as-writable-ports, and QoS come later.

use proc_macro::TokenStream;
use quote::quote;
use syn::{
    parse_macro_input, punctuated::Punctuated, Expr, FnArg, ImplItem, ItemImpl, Lit, MetaNameValue,
    Pat, ReturnType, Token, Type,
};

#[proc_macro_attribute]
pub fn node(attr: TokenStream, item: TokenStream) -> TokenStream {
    // ---- parse attribute args: id = "...", out = "..." ----
    let args = parse_macro_input!(
        attr with Punctuated::<MetaNameValue, Token![,]>::parse_terminated
    );
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

    // ---- parse the impl block and find/rename `process` ----
    let mut impl_block = parse_macro_input!(item as ItemImpl);

    let mut sig = None;
    for it in &mut impl_block.items {
        if let ImplItem::Fn(f) = it {
            if f.sig.ident == "process" {
                // Clone the signature WITH its `#[param]` attrs intact (we read defaults from
                // them below), then rename the method and strip the attrs from the re-emitted
                // copy — a custom attribute on a fn parameter would otherwise fail to compile.
                sig = Some(f.sig.clone());
                f.sig.ident = syn::Ident::new("__node_run", f.sig.ident.span());
                for arg in f.sig.inputs.iter_mut() {
                    if let FnArg::Typed(pt) = arg {
                        pt.attrs.retain(|a| !a.path().is_ident("param"));
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

    // ---- inputs: each non-receiver param -> (name, element type) ----
    let mut in_names: Vec<String> = Vec::new();
    let mut in_types: Vec<Type> = Vec::new();
    let mut in_defaults: Vec<Option<syn::Expr>> = Vec::new();
    for arg in sig.inputs.iter() {
        let FnArg::Typed(pt) = arg else { continue }; // skip &self
        let name = match &*pt.pat {
            Pat::Ident(pi) => pi.ident.to_string(),
            other => {
                return syn::Error::new_spanned(other, "#[node] params must be plain identifiers")
                    .to_compile_error()
                    .into()
            }
        };
        // accept `&T` (preferred) or bare `T`; record the element type T
        let elem = match &*pt.ty {
            Type::Reference(r) => (*r.elem).clone(),
            other => other.clone(),
        };
        // `#[param(default = <expr>)]` marks this input as a parameter with a fallback value.
        let mut default = None;
        for a in &pt.attrs {
            if a.path().is_ident("param") {
                match a.parse_args::<MetaNameValue>() {
                    Ok(nv) if nv.path.is_ident("default") => default = Some(nv.value),
                    _ => {
                        return syn::Error::new_spanned(a, "#[param] expects `default = <expr>`")
                            .to_compile_error()
                            .into()
                    }
                }
            }
        }
        in_names.push(name);
        in_types.push(elem);
        in_defaults.push(default);
    }

    let out_ty: Option<Type> = match &sig.output {
        ReturnType::Default => None,
        ReturnType::Type(_, t) => Some((**t).clone()),
    };

    // ---- generate fragments ----
    let in_ports = in_names
        .iter()
        .zip(in_types.iter())
        .zip(in_defaults.iter())
        .map(|((n, t), default)| match default {
            Some(expr) => quote! {
                ::octans_core::PortSpec::with_default(
                    #n,
                    <#t as ::octans_core::RegisteredType>::type_spec(),
                    ::octans_core::Value::new::<#t>(#expr),
                )
            },
            None => quote! {
                ::octans_core::PortSpec::new(#n, <#t as ::octans_core::RegisteredType>::type_spec())
            },
        });

    let outputs_method = match &out_ty {
        Some(t) => quote! {
            vec![ ::octans_core::PortSpec::new(#out_name, <#t as ::octans_core::RegisteredType>::type_spec()) ]
        },
        None => quote! { ::std::vec::Vec::new() },
    };

    let binds = in_names.iter().zip(in_types.iter()).map(|(n, t)| {
        let var = syn::Ident::new(n, proc_macro2::Span::call_site());
        quote! { let #var = inputs.get::<#t>(#n); }
    });
    let call_args = in_names
        .iter()
        .map(|n| syn::Ident::new(n, proc_macro2::Span::call_site()));
    let run = quote! { self.__node_run( #( #call_args ),* ) };
    let body_tail = match &out_ty {
        Some(_) => quote! { let __ret = #run; outputs.set(#out_name, __ret); },
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
            fn process(&self, inputs: &::octans_core::Inputs, outputs: &mut ::octans_core::Outputs) {
                #( #binds )*
                #body_tail
            }
        }
    }
    .into()
}
