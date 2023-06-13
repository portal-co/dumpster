/*
   dumpster, a cycle-tracking garbage collector for Rust.
   Copyright (C) 2023 Clayton Ramsey.

   This program is free software: you can redistribute it and/or modify
   it under the terms of the GNU General Public License as published by
   the Free Software Foundation, either version 3 of the License, or
   (at your option) any later version.

   This program is distributed in the hope that it will be useful,
   but WITHOUT ANY WARRANTY; without even the implied warranty of
   MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
   GNU General Public License for more details.

   You should have received a copy of the GNU General Public License
   along with this program.  If not, see <http://www.gnu.org/licenses/>.
*/

use proc_macro2::TokenStream;
use quote::{quote, quote_spanned};
use syn::{
    parse_macro_input, parse_quote, spanned::Spanned, Data, DeriveInput, Fields, GenericParam,
    Generics, Ident, Index,
};

#[proc_macro_derive(Collectable)]
/// The macro for implementing [`dumpster::Collectable`] on an arbitrary type.
/// This implementation assumes that all fields of the structure or enum deriving Collectable have
/// also implemented Collectable.
pub fn derive_collectable(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    // name of the type being implemented
    let name = &input.ident;

    // generic parameters of the type being implemented
    let generics = add_trait_bounds(input.generics);
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let (generate_graph, destroy_gcs) = delegate_methods(name, &input.data);

    let generated = quote! {
        unsafe impl #impl_generics dumpster::Collectable for #name #ty_generics #where_clause {
            #[inline]
            fn add_to_ref_graph(
                &self,
                self_ref: dumpster::AllocationId,
                ref_graph: &mut dumpster::RefGraph,
            ) {
                #generate_graph
            }

            #[inline]
            unsafe fn destroy_gcs(&mut self) {
                #destroy_gcs
            }
        }
    };

    generated.into()
}

/// Collect the trait bounds for some generic expression.
fn add_trait_bounds(mut generics: Generics) -> Generics {
    for param in &mut generics.params {
        if let GenericParam::Type(ref mut type_param) = *param {
            type_param.bounds.push(parse_quote!(heapsize::HeapSize));
        }
    }
    generics
}

/// Generate method implementations for [`Collectable`] for some data type.
///
/// Returns a pair containing the method body for [`Collectable::add_to_ref_graph`] and
/// [`Collectable::destroy_gcs`].
fn delegate_methods(name: &Ident, data: &Data) -> (TokenStream, TokenStream) {
    match data {
        Data::Struct(data) => match data.fields {
            Fields::Named(ref f) => {
                let delegate_graph = f.named.iter().map(|f| {
                    let name = &f.ident;
                    quote_spanned! {f.span() =>
                        dumpster::Collectable::add_to_ref_graph(
                            &self.#name,
                            self_ref,
                            ref_graph
                        );
                    }
                });

                let delegate_destroy = f.named.iter().map(|f| {
                    let name = &f.ident;
                    quote_spanned! {f.span() =>
                        dumpster::Collectable::destroy_gcs(
                            &mut self.#name,
                        );
                    }
                });
                (
                    quote! { #(#delegate_graph)* },
                    quote! { #(#delegate_destroy)* },
                )
            }
            Fields::Unnamed(ref f) => {
                let delegate_graph = f.unnamed.iter().enumerate().map(|(i, f)| {
                    let index = Index::from(i);
                    quote_spanned! {f.span() =>
                        dumpster::Collectable::add_to_ref_graph(
                            &self.#index,
                            self_ref,
                            ref_graph
                        );
                    }
                });
                let delegate_destroy = f.unnamed.iter().enumerate().map(|(i, f)| {
                    let index = Index::from(i);
                    quote_spanned! {f.span() =>
                        dumpster::Collectable::add_to_ref_graph(&mut self.#index);
                    }
                });
                (
                    quote! { #(#delegate_graph)* },
                    quote! { #(#delegate_destroy)* },
                )
            }
            Fields::Unit => (TokenStream::new(), TokenStream::new()),
        },
        Data::Enum(e) => {
            let mut delegate_graph = TokenStream::new();
            let mut delegate_destroy = TokenStream::new();
            for var in e.variants.iter() {
                let var_name = &var.ident;

                match &var.fields {
                    Fields::Named(n) => {
                        let mut binding = TokenStream::new();
                        let mut execution_graph = TokenStream::new();
                        let mut execution_destroy = TokenStream::new();
                        for (i, name) in n.named.iter().enumerate() {
                            let field_name = format!("field{i}");
                            let field_ident = name.ident.as_ref().unwrap();
                            if i == 0 {
                                binding.extend(quote! {
                                    #field_ident: #field_name
                                });
                            } else {
                                binding.extend(quote! {
                                    , #field_ident: #field_name
                                });
                            }

                            execution_graph.extend(quote! {
                                dumpster::Collectable::add_to_ref_graph(
                                    #field_name,
                                    self_ref,
                                    ref_graph
                                );
                            });

                            execution_destroy.extend(quote! {
                                dumpster::Collectable::destroy_gcs(
                                    #field_name,
                                );
                            })
                        }

                        delegate_graph
                            .extend(quote! {#name::#var_name{#binding} => {#execution_graph}});
                        delegate_destroy
                            .extend(quote! {#name::#var_name{#binding} => {#execution_destroy}});
                    }
                    Fields::Unnamed(u) => {
                        let mut binding = TokenStream::new();
                        let mut execution_graph = TokenStream::new();
                        let mut execution_destroy = TokenStream::new();
                        for (i, _) in u.unnamed.iter().enumerate() {
                            let field_name = format!("field{i}");
                            if i == 0 {
                                binding.extend(quote! {
                                    #field_name
                                });
                            } else {
                                binding.extend(quote! {
                                    , #field_name
                                });
                            }

                            execution_graph.extend(quote! {
                                dumpster::Collectable::add_to_ref_graph(
                                    #field_name,
                                    self_ref,
                                    ref_graph
                                );
                            });

                            execution_destroy.extend(quote! {
                                dumpster::Collectable::destroy_gcs(#field_name)
                            });
                        }

                        delegate_graph
                            .extend(quote! {#name::#var_name(#binding) => {#execution_graph}});
                        delegate_destroy
                            .extend(quote! {#name::#var_name(#binding) => {#execution_destroy}});
                    }
                    Fields::Unit => {
                        delegate_graph.extend(quote! {#name::#var_name => (),});
                        delegate_destroy.extend(quote! {#name::#var_name => (),});
                    }
                }
            }

            (delegate_graph, delegate_destroy)
        }
        Data::Union(u) => {
            let stream = quote_spanned! {
                u.union_token.span => compile_error!("`Collectable` must be manually implemented for unions");
            };
            (stream.clone(), stream)
        }
    }
}
